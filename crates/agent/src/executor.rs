use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use gui_use_host::server::{self, ControlRequest};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tracing::Instrument;

use crate::{
    logging::RpcTrace,
    worker::{self, CODE_LIMIT, FRAME_LIMIT},
};

#[derive(Clone)]
pub struct Config {
    pub worker: PathBuf,
    pub directory: PathBuf,
    pub device: String,
    pub artifacts: PathBuf,
    pub allow_actions: bool,
    pub allow_notification_reply: bool,
    pub timeout_ms: u64,
}

#[derive(Debug, Default, Serialize)]
pub struct Stats {
    pub rpc_calls: usize,
    pub rpc_response_bytes: usize,
    pub model_visible_bytes: usize,
    pub elapsed_ms: u128,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub ok: bool,
    pub result: Value,
    pub output: Vec<String>,
    pub truncated: bool,
    pub error: Option<String>,
    pub artifacts: Vec<String>,
    pub stats: Stats,
}

impl Report {
    fn failure(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            result: Value::Null,
            output: vec![],
            truncated: false,
            error: Some(error.into()),
            artifacts: vec![],
            stats: Stats::default(),
        }
    }
}

pub struct Executor {
    pub config: Config,
    state: Value,
    screenshot_index: usize,
}

impl Executor {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            state: json!({}),
            screenshot_index: 0,
        }
    }

    #[tracing::instrument(name = "ptc", skip_all)]
    pub async fn execute(&mut self, code: &str) -> Report {
        let started = Instant::now();
        if code.len() > CODE_LIMIT {
            return Report::failure("Code exceeds 32 KiB");
        }
        let mut stats = Stats::default();
        let mut artifacts = Vec::new();
        let deadline = Duration::from_millis(self.config.timeout_ms);
        let execution =
            tokio::time::timeout(deadline, self.run_worker(code, &mut stats, &mut artifacts)).await;
        let mut report = match execution {
            Ok(Ok(output)) => {
                self.state = output.state;
                Report {
                    ok: output.error.is_none(),
                    result: output.result,
                    output: output.output,
                    truncated: output.truncated,
                    error: output.error,
                    artifacts: vec![],
                    stats: Stats::default(),
                }
            }
            Ok(Err(_)) => Report::failure("PTC worker failed or exceeded a frame/resource limit"),
            Err(_) => Report::failure(
                "PTC deadline exceeded; worker terminated. Earlier actions are NOT rolled back; inspect before retrying.",
            ),
        };
        stats.elapsed_ms = started.elapsed().as_millis();
        report.artifacts = artifacts;
        report.stats = stats;
        loop {
            let length = serde_json::to_vec(&report).unwrap_or_default().len();
            if length == report.stats.model_visible_bytes {
                break;
            }
            report.stats.model_visible_bytes = length;
        }
        tracing::info!(
            event = "ptc.completed",
            ok = report.ok,
            rpc_calls = report.stats.rpc_calls,
            raw_bytes = report.stats.rpc_response_bytes,
            result_bytes = report.stats.model_visible_bytes,
            elapsed_ms = report.stats.elapsed_ms as u64,
            error = report.error.as_deref(),
            "PTC execution completed"
        );
        report
    }

    async fn run_worker(
        &mut self,
        code: &str,
        stats: &mut Stats,
        artifacts: &mut Vec<String>,
    ) -> Result<worker::Output> {
        let mut child = Command::new(&self.config.worker)
            .arg("ptc-worker")
            .env_clear()
            .current_dir("/")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()?;
        let mut stdin = child.stdin.take().context("Worker stdin")?;
        let mut stdout = BufReader::new(child.stdout.take().context("Worker stdout")?);
        let input = worker::Input {
            code: code.into(),
            state: self.state.clone(),
            timeout_ms: self.config.timeout_ms,
        };
        stdin.write_all(&serde_json::to_vec(&input)?).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        loop {
            let frame = read_frame(&mut stdout, FRAME_LIMIT).await?;
            let value: Value = serde_json::from_slice(&frame)?;
            match value["kind"].as_str() {
                Some("rpc") => {
                    let method = value["method"].as_str().context("Missing method")?;
                    let params = value["params"].clone();
                    let mut trace = RpcTrace::new(stats.rpc_calls + 1, method, &params);
                    let called = self
                        .call(method, params, stats, artifacts)
                        .instrument(trace.span.clone())
                        .await;
                    trace.finish(&called);
                    let response = match called {
                        Ok(response) => response,
                        Err(error) => json!({"error":{"code":-32000,"message":error.to_string()}}),
                    };
                    stdin.write_all(&serde_json::to_vec(&response)?).await?;
                    stdin.write_all(b"\n").await?;
                    stdin.flush().await?;
                }
                Some("done") => {
                    let output = serde_json::from_value(value["value"].clone())?;
                    ensure!(
                        child.wait().await?.success(),
                        "Worker exited unsuccessfully"
                    );
                    return Ok(output);
                }
                _ => anyhow::bail!("Unknown worker frame"),
            }
        }
    }

    async fn call(
        &mut self,
        method: &str,
        params: Value,
        stats: &mut Stats,
        artifacts: &mut Vec<String>,
    ) -> Result<Value> {
        // Statistics only: RPC counts never terminate a task or code execution.
        stats.rpc_calls += 1;
        ensure!(
            params.is_object() && params.to_string().len() <= CODE_LIMIT,
            "Expected a bounded parameter object"
        );
        // Fail closed when the Rust service gains new methods: each capability needs
        // an explicit permission classification, not just a generated JS wrapper.
        let mutating = match method {
            "app.list" | "ui.snapshot" | "ui.wait" | "ui.screenshot" | "notification.list"
            | "host.devices" | "host.events" => false,
            "app.launch" | "ui.act" | "ui.gesture" | "notification.reply" => true,
            _ => anyhow::bail!("Method is not exposed to PTC"),
        };
        ensure!(
            !mutating || self.config.allow_actions,
            "Actions are disabled; user must enable --allow-actions"
        );
        ensure!(
            method != "notification.reply" || self.config.allow_notification_reply,
            "Notification replies are disabled; user must enable --allow-notification-reply"
        );
        let mut response = server::control(
            &self.config.directory,
            ControlRequest {
                method: method.into(),
                params,
                device: Some(self.config.device.clone()),
                timeout_ms: Some(self.config.timeout_ms.min(70000)),
            },
        )
        .await
        .context("Local host request failed")?;
        stats.rpc_response_bytes += serde_json::to_vec(&response)?.len();
        if let Some(binary) = response
            .as_object_mut()
            .and_then(|object| object.remove("binary"))
        {
            ensure!(method == "ui.screenshot", "Unexpected binary response");
            let bytes = STANDARD.decode(binary.as_str().context("Invalid image bytes")?)?;
            ensure!(
                bytes.len() <= 8 * 1024 * 1024 && bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
                "Invalid PNG artifact"
            );
            if !self.config.artifacts.exists() {
                std::fs::create_dir_all(&self.config.artifacts)?;
                std::fs::set_permissions(
                    &self.config.artifacts,
                    std::fs::Permissions::from_mode(0o700),
                )?;
            }
            self.screenshot_index += 1;
            let name = format!("screenshot-{}.png", self.screenshot_index);
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .mode(0o600)
                .open(self.config.artifacts.join(&name))?;
            std::io::Write::write_all(&mut file, &bytes)?;
            response["result"]["artifact"] = json!(name);
            artifacts.push(name);
        }
        Ok(response)
    }
}

/// Bounded while reading, not after allocating an unbounded read_line buffer.
pub async fn read_frame<R: AsyncBufRead + Unpin>(reader: &mut R, limit: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        ensure!(!available.is_empty(), "Unexpected EOF");
        let end = available.iter().position(|byte| *byte == b'\n');
        let take = end.map_or(available.len(), |index| index + 1);
        ensure!(bytes.len() + take <= limit, "Frame exceeds limit");
        bytes.extend_from_slice(&available[..take]);
        reader.consume(take);
        if end.is_some() {
            return Ok(bytes);
        }
    }
}
