//! Console diagnostics go to stderr; stdout remains the result/worker JSON channel.
use std::io::IsTerminal;
use std::time::Instant;

use serde_json::{Value, json};
use tracing::Span;
use tracing_subscriber::EnvFilter;

pub fn init() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("warn,gui_use_agent=info,gui_use_host=info")),
        )
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_target(false)
        .init();
}

/// Never dump credentials or image bytes, even when detailed RPC logging is enabled.
pub fn redacted(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    let sensitive = matches!(
                        key.to_ascii_lowercase().as_str(),
                        "token"
                            | "api_key"
                            | "authorization"
                            | "password"
                            | "pin"
                            | "certificate_sha256"
                            | "binary"
                    );
                    (
                        key.clone(),
                        if sensitive {
                            json!("[redacted]")
                        } else {
                            redacted(value)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redacted).collect()),
        other => other.clone(),
    }
}

fn response_summary(method: &str, response: &Value) -> Value {
    if response.get("error").is_some() {
        return redacted(response);
    }
    let result = &response["result"];
    match method {
        "ui.snapshot" => json!({"seq":result["seq"],"package_name":result["package_name"],
            "activity":result["activity"],"truncated":result["truncated"],
            "node_count":result["nodes"].as_array().map(Vec::len)}),
        "app.list" => json!({"app_count":result["apps"].as_array().map(Vec::len)}),
        "notification.list" => {
            json!({"notification_count":result["notifications"].as_array().map(Vec::len)})
        }
        "host.events" => json!({"event_count":result.as_array().map(Vec::len)}),
        _ => redacted(result),
    }
}

/// Also emit a terminal event if an RPC future is dropped by timeout or Ctrl-C.
pub struct RpcTrace {
    pub span: Span,
    method: String,
    started: Instant,
    finished: bool,
}

impl RpcTrace {
    pub fn new(index: usize, method: &str, params: &Value) -> Self {
        let span = tracing::info_span!("rpc", rpc = index, method);
        tracing::info!(parent: &span, event = "rpc.request", params = %redacted(params), "RPC request");
        Self {
            span,
            method: method.into(),
            started: Instant::now(),
            finished: false,
        }
    }

    pub fn finish(&mut self, response: &anyhow::Result<Value>) {
        self.finished = true;
        let elapsed_ms = self.started.elapsed().as_millis() as u64;
        match response {
            Ok(response) if response.get("error").is_none() => {
                tracing::info!(parent: &self.span, event = "rpc.response", elapsed_ms,
                    result = %response_summary(&self.method, response), "RPC completed");
                tracing::debug!(parent: &self.span, event = "rpc.payload", response = %redacted(response), "RPC response payload");
            }
            Ok(response) => {
                tracing::warn!(parent: &self.span, event = "rpc.error", elapsed_ms,
                    error = %redacted(&response["error"]), "RPC failed");
            }
            Err(error) => {
                tracing::warn!(parent: &self.span, event = "rpc.error", elapsed_ms, error = %error, "RPC failed");
            }
        }
    }
}

impl Drop for RpcTrace {
    fn drop(&mut self) {
        if !self.finished {
            tracing::warn!(parent: &self.span, event = "rpc.cancelled",
                elapsed_ms = self.started.elapsed().as_millis() as u64, "RPC interrupted");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_and_binary_are_redacted_recursively_but_rpc_arguments_remain_visible() {
        assert_eq!(
            redacted(
                &json!({"action":"set_text","text":"hello","nested":[{"TOKEN":"secret","binary":"png"}]})
            ),
            json!({"action":"set_text","text":"hello","nested":[{"TOKEN":"[redacted]","binary":"[redacted]"}]})
        );
    }
}
