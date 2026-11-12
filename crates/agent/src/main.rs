use std::path::PathBuf;

use anyhow::{Result, ensure};
use clap::{Parser, Subcommand};
use gui_use_agent::{
    agent::{self, Limits},
    api, bootstrap,
    executor::{Config, Executor},
    model::Model,
    worker,
};

#[derive(Parser)]
#[command(
    version,
    about = "Android agent with one bounded JavaScript programmatic-tool-calling tool"
)]
struct Args {
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
    #[arg(long, global = true)]
    device: Option<String>,
    /// Select a USB phone by adb serial; otherwise reuse one connection or auto-detect one installed phone.
    #[arg(long, global = true, env = "GUI_USE_SERIAL")]
    serial: Option<String>,
    /// Enable app launch, UI actions and gestures. Android allowlisting still applies.
    #[arg(long, global = true)]
    allow_actions: bool,
    /// Additionally permit notification replies; not a blanket approval to send messages.
    #[arg(long, global = true)]
    allow_notification_reply: bool,
    #[arg(long, global = true, default_value_t = 30000)]
    code_timeout_ms: u64,
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Run a natural-language task against an OpenAI-compatible chat-completions API.
    Run {
        task: String,
        #[arg(
            long,
            env = "GUI_USE_BASE_URL",
            default_value = "https://api.deepseek.com"
        )]
        base_url: String,
        #[arg(long, env = "GUI_USE_MODEL", default_value = "deepseek-flash")]
        model: String,
        /// Read a token privately; never pass the token itself as a CLI argument.
        #[arg(long, env = "GUI_USE_API_KEY_FILE")]
        api_key_file: Option<PathBuf>,
        /// Provider-specific thinking switch (supported by DeepSeek); omitted by default.
        #[arg(long, env = "GUI_USE_THINKING", value_enum)]
        thinking: Option<Thinking>,
        /// Optional step limit within this user turn. Omit to run until completion or Ctrl-C.
        #[arg(long, visible_alias = "max-turns")]
        max_steps: Option<usize>,
    },
    /// Execute a local PTC program without a model (same runtime and permission policy).
    Exec {
        #[arg(long)]
        file: PathBuf,
    },
    /// Inspect the Rust-derived API without connecting to a device or model.
    Api { method: Option<String> },
    /// Automatically prepare host/USB connection and keep it available until Ctrl-C.
    Connect,
    #[command(hide = true)]
    PtcWorker,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum Thinking {
    Enabled,
    Disabled,
}

fn main() -> Result<()> {
    let args = Args::parse();
    if matches!(args.command, Action::PtcWorker) {
        return worker::run();
    }
    gui_use_agent::logging::init();
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(run(args))
}

async fn run(args: Args) -> Result<()> {
    if let Action::Api { method } = args.command {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &method.map_or_else(api::catalog, |name| api::help(&name))
            )?
        );
        return Ok(());
    }
    ensure!(
        (100..=120000).contains(&args.code_timeout_ms),
        "Code timeout must be between 100 and 120000 ms"
    );
    let directory = args.state_dir.unwrap_or_else(|| {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into()))
                    .join(".local/state")
            })
            .join("gui-use")
    });
    let run_id = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
        std::process::id()
    );
    let artifacts = directory.join("agent-artifacts").join(run_id);
    let work = async {
        // Validate the model configuration before touching a phone; don't pay for a
        // model request until host/device preparation has succeeded.
        let model = if let Action::Run {
            base_url,
            model,
            api_key_file,
            thinking,
            max_steps,
            ..
        } = &args.command
        {
            ensure!(
                max_steps.is_none_or(|steps| steps > 0),
                "max-steps must be positive when specified"
            );
            let key = match api_key_file {
                Some(path) => {
                    ensure!(
                        std::fs::metadata(path)?.len() <= 16384,
                        "API key file exceeds limit"
                    );
                    Some(std::fs::read_to_string(path)?.trim().to_owned())
                }
                None => std::env::var("GUI_USE_API_KEY")
                    .or_else(|_| std::env::var("DEEPSEEK_API_KEY"))
                    .ok(),
            };
            if base_url == "https://api.deepseek.com" {
                ensure!(
                    key.as_ref()
                        .is_some_and(|key| !key.is_empty() && key != "DEEPSEEK_API_KEY"),
                    "请先设置 DEEPSEEK_API_KEY 或 GUI_USE_API_KEY（需要变量内容，不是变量名称）"
                );
            }
            let mut configured = Model::new(base_url, model.clone(), key)?;
            if let Some(thinking) = thinking {
                configured = configured.with_thinking(matches!(thinking, Thinking::Enabled));
            } else if base_url == "https://api.deepseek.com" {
                configured = configured.with_thinking(false);
            }
            Some(configured)
        } else {
            None
        };
        let connection =
            bootstrap::prepare(&directory, args.device.as_deref(), args.serial.as_deref()).await?;
        if matches!(args.command, Action::Connect) {
            tracing::info!("已连接，手机服务可在后台运行。按 Ctrl-C 退出电脑端 host。");
            std::future::pending::<()>().await;
        }
        let mut executor = Executor::new(Config {
            worker: std::env::current_exe()?,
            directory,
            device: connection.device.clone(),
            artifacts: artifacts.clone(),
            allow_actions: args.allow_actions,
            allow_notification_reply: args.allow_notification_reply,
            timeout_ms: args.code_timeout_ms,
        });
        match args.command {
            Action::Exec { file } => {
                ensure!(
                    std::fs::metadata(&file)?.len() <= worker::CODE_LIMIT as u64,
                    "Code file exceeds 32 KiB"
                );
                let code = std::fs::read_to_string(file)?;
                let report = executor.execute(&code).await;
                println!("{}", serde_json::to_string_pretty(&report)?);
                ensure!(report.ok, "PTC execution did not complete successfully");
            }
            Action::Run {
                task, max_steps, ..
            } => {
                let outcome = agent::run(
                    model.as_ref().expect("configured model"),
                    &mut executor,
                    &task,
                    Limits { steps: max_steps },
                )
                .await?;
                println!("{}", serde_json::to_string_pretty(&outcome)?);
            }
            _ => unreachable!(),
        }
        if artifacts.exists() {
            tracing::info!(path = %artifacts.display(), "Private screenshot artifacts");
        }
        Ok(())
    };
    tokio::select! {
        result = work => result,
        _ = tokio::signal::ctrl_c() => anyhow::bail!("Interrupted; PTC worker terminated. Device side effects are not rolled back."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_does_not_limit_steps_unless_explicitly_requested() {
        let args = Args::try_parse_from(["gui-use-agent", "run", "list apps"]).unwrap();
        assert!(matches!(
            args.command,
            Action::Run {
                max_steps: None,
                ..
            }
        ));
        for flag in ["--max-steps", "--max-turns"] {
            let args = Args::try_parse_from(["gui-use-agent", "run", flag, "100000", "list apps"])
                .unwrap();
            assert!(matches!(
                args.command,
                Action::Run {
                    max_steps: Some(100000),
                    ..
                }
            ));
        }
    }
}
