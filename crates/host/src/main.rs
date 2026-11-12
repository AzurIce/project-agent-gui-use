use std::io::Write;
use std::net::SocketAddr;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use clap::{Parser, Subcommand};
use gui_use_host::client::LocalTransport;
use gui_use_host::{
    identity,
    server::{self, ControlRequest},
};
use gui_use_protocol::DeviceClient;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    version,
    about = "Paired reverse-WebSocket host for Android automation"
)]
struct Args {
    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a private TLS identity and pairing file. Never overwrites an identity.
    Init {
        #[arg(long, default_value = "wss://localhost:8765")]
        endpoint: String,
    },
    /// Accept authenticated device connections and local CLI commands.
    Serve {
        #[arg(long, default_value = "127.0.0.1:8765")]
        listen: SocketAddr,
    },
    Devices,
    /// List applications through the generated, typed Rust DeviceClient.
    Apps {
        #[arg(long)]
        include_system: bool,
        #[arg(long)]
        device: Option<String>,
    },
    /// Read the bounded in-memory event buffer (payloads may contain private data).
    Events {
        #[arg(long)]
        device: Option<String>,
    },
    /// Invoke a method with a named JSON parameter object.
    Call {
        method: String,
        #[arg(long, default_value = "{}")]
        params: String,
        #[arg(long)]
        device: Option<String>,
        #[arg(long, default_value_t = 10000)]
        timeout_ms: u64,
        /// Save screenshot PNG bytes to a new file; existing files are not overwritten.
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let directory = args.state_dir.unwrap_or_else(|| {
        PathBuf::from(std::env::var_os("XDG_STATE_HOME").unwrap_or_else(|| {
            let mut home = PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into()));
            home.push(".local/state");
            home.into_os_string()
        }))
        .join("gui-use")
    });
    match args.command {
        Command::Init { endpoint } => {
            anyhow::ensure!(
                endpoint.starts_with("wss://"),
                "Only wss:// endpoints are accepted"
            );
            identity::initialize(&directory, &endpoint)?;
            println!(
                "Pairing configuration saved privately to {}",
                directory.join("pairing.json").display()
            );
        }
        Command::Serve { listen } => {
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            tokio::select! { result = server::run(&directory, listen) => result?, _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
        }
        Command::Apps {
            include_system,
            device,
        } => {
            let client = DeviceClient::new(LocalTransport::new(&directory, device));
            let result = client.app_list(Some(include_system)).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"result":result}))?
            );
        }
        command => {
            let (request, output) = match command {
                Command::Devices => (
                    ControlRequest {
                        method: "host.devices".into(),
                        params: json!({}),
                        device: None,
                        timeout_ms: None,
                    },
                    None,
                ),
                Command::Events { device } => (
                    ControlRequest {
                        method: "host.events".into(),
                        params: json!({}),
                        device,
                        timeout_ms: None,
                    },
                    None,
                ),
                Command::Call {
                    method,
                    params,
                    device,
                    timeout_ms,
                    output,
                } => (
                    ControlRequest {
                        method,
                        params: serde_json::from_str(&params)
                            .context("--params must contain JSON")?,
                        device,
                        timeout_ms: Some(timeout_ms),
                    },
                    output,
                ),
                _ => unreachable!(),
            };
            let mut response = server::control(&directory, request).await?;
            if let Some(binary) = response
                .as_object_mut()
                .and_then(|object| object.remove("binary"))
            {
                let bytes = STANDARD.decode(binary.as_str().context("Invalid binary response")?)?;
                response["binary_bytes"] = json!(bytes.len());
                if let Some(path) = output {
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(&path)?
                        .write_all(&bytes)?;
                    response["output"] = json!(path);
                }
            }
            println!("{}", serde_json::to_string_pretty(&response)?);
            if response
                .get("error")
                .is_some_and(|value| value != &Value::Null)
            {
                std::process::exit(1);
            }
        }
    }
    Ok(())
}
