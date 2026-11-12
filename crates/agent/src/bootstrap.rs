//! Local USB development bootstrap. Host lifecycle belongs to the foreground CLI;
//! Android service lifecycle belongs to the phone's Start/Stop switch.
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use gui_use_host::{
    identity,
    server::{self, ControlRequest},
};
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const PACKAGE: &str = "dev.azurice.guiuse";

pub struct Connection {
    pub device: String,
    _host: Option<OwnedHost>,
}

struct OwnedHost(tokio::task::JoinHandle<Result<()>>);
impl Drop for OwnedHost {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn devices(directory: &Path) -> Result<Vec<Value>> {
    let value = tokio::time::timeout(
        Duration::from_secs(2),
        server::control(
            directory,
            ControlRequest {
                method: "host.devices".into(),
                params: json!({}),
                device: None,
                timeout_ms: None,
            },
        ),
    )
    .await??;
    value["result"]
        .as_array()
        .cloned()
        .context("Host did not return a device list")
}

fn pick_connected(devices: &[Value], requested: Option<&str>) -> Result<Option<String>> {
    if let Some(id) = requested {
        return Ok(devices
            .iter()
            .find(|device| device["id"] == id)
            .map(|_| id.to_owned()));
    }
    ensure!(
        devices.len() <= 1,
        "多台手机已连接，请使用 --device 指定，或 --serial 选择 USB 手机"
    );
    Ok(devices
        .first()
        .and_then(|device| device["id"].as_str())
        .map(str::to_owned))
}

pub async fn prepare(
    directory: &Path,
    requested: Option<&str>,
    serial: Option<&str>,
) -> Result<Connection> {
    let adb = std::env::var_os("GUI_USE_ADB")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("adb"));
    prepare_with_adb(directory, requested, serial, &adb).await
}

async fn prepare_with_adb(
    directory: &Path,
    requested: Option<&str>,
    serial: Option<&str>,
    adb: &Path,
) -> Result<Connection> {
    let mut host = None;
    if devices(directory).await.is_err() {
        if !directory.join("pairing.json").exists() {
            tracing::info!("首次运行：自动创建本地连接配置");
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
            let port = listener.local_addr()?.port();
            identity::initialize(directory, &format!("wss://localhost:{port}"))?;
        }
        let (pairing, _) = identity::load(directory)?;
        let url = reqwest::Url::parse(&pairing.endpoint)?;
        let port = url
            .port_or_known_default()
            .context("Host endpoint has no port")?;
        let host_directory = directory.to_path_buf();
        let owned = OwnedHost(tokio::spawn(async move {
            server::run(&host_directory, ([127, 0, 0, 1], port).into()).await
        }));
        tracing::info!("自动启动本地 host（任务结束后退出）");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if devices(directory).await.is_ok() {
                    break;
                }
                if owned.0.is_finished() {
                    bail!("本地 host 未能启动，请检查端口是否已占用");
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok::<_, anyhow::Error>(())
        })
        .await
        .context("Host 启动超时；请确认端口未被其他程序占用")??;
        host = Some(owned);
    }
    if serial.is_none() {
        if let Some(device) = pick_connected(&devices(directory).await?, requested)? {
            tracing::info!("复用已连接的手机");
            return Ok(Connection {
                device,
                _host: host,
            });
        }
        ensure!(
            requested.is_none(),
            "指定的 --device 尚未连接。可省略 --device 自动接入 USB 手机，或用 --serial 指定手机"
        );
    }
    let serial = match serial {
        Some(serial) => serial.to_owned(),
        None => {
            let listing = adb_command(adb, None, &["devices", "-l"], None).await?;
            let candidates = usb_devices(&listing);
            let mut installed = Vec::new();
            for (serial, label) in candidates {
                if adb_command(adb, Some(&serial), &["shell", "pm", "path", PACKAGE], None)
                    .await
                    .is_ok_and(|text| text.contains("package:"))
                {
                    installed.push((serial, label));
                }
            }
            ensure!(
                !installed.is_empty(),
                "没有找到已安装 GUI Use 的 USB 手机。请连接 USB、允许 USB 调试并安装最新 debug APK"
            );
            if installed.len() != 1 {
                let choices = installed
                    .iter()
                    .map(|(serial, label)| format!("{label}: --serial {serial}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                bail!("找到多台手机，请选择一台：\n{choices}");
            }
            let (serial, label) = installed.remove(0);
            tracing::info!(model = %label, serial = %serial, "发现手机");
            serial
        }
    };
    // Don't steal focus from the app the user wants to operate on every task.
    // Opening the control panel is only needed to start an inactive/stopped app.
    let services = adb_command(
        adb,
        Some(&serial),
        &["shell", "dumpsys", "activity", "services", PACKAGE],
        None,
    )
    .await?;
    if needs_control_activity(&services) {
        adb_command(
            adb,
            Some(&serial),
            &[
                "shell",
                "am",
                "start",
                "-n",
                "dev.azurice.guiuse/.MainActivity",
            ],
            None,
        )
        .await?;
    }
    let capability = broadcast(adb, &serial, "capabilities").await?;
    ensure!(
        capability.contains("usb-import"),
        "手机 GUI Use 需要更新：请安装本项目最新 debug APK，以支持 USB 自动配置"
    );
    let (mut pairing, _) = identity::load(directory)?;
    let port = reqwest::Url::parse(&pairing.endpoint)?
        .port_or_known_default()
        .context("Invalid host port")?;
    let mapping = format!("tcp:{port}");
    let reverse = adb_command(adb, Some(&serial), &["reverse", "--list"], None).await?;
    ensure!(
        reverse.lines().all(|line| {
            let columns = line.split_whitespace().collect::<Vec<_>>();
            columns.len() < 3 || columns[1] != mapping || columns[2] == mapping
        }),
        "此 USB 端口已有其他映射，请先处理冲突，未覆盖原映射"
    );
    adb_command(adb, Some(&serial), &["reverse", &mapping, &mapping], None).await?;
    pairing.endpoint = format!("wss://localhost:{port}");
    // Credentials travel through stdin, not shell arguments or process listings.
    adb_command(
        adb,
        Some(&serial),
        &[
            "shell",
            "run-as",
            PACKAGE,
            "sh",
            "-c",
            "'mkdir -p files && cat > files/usb-pairing.json'",
        ],
        Some(serde_json::to_vec(&pairing)?),
    )
    .await?;
    let imported = broadcast(adb, &serial, "import").await?;
    let id =
        broadcast_data(&imported).context("手机未接受 USB 配置，请更新 debug APK 并再次启动")?;
    ensure!(
        imported.contains("result=1") && !id.is_empty() && id.len() <= 128,
        "USB 配置未成功导入"
    );
    if let Some(requested) = requested {
        ensure!(id == requested, "USB 手机与 --device 指定的设备不一致");
    }
    tracing::info!("已自动配置手机并启动服务，等待连接…");
    let device = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if devices(directory)
                .await?
                .iter()
                .any(|device| device["id"] == id)
            {
                return Ok::<_, anyhow::Error>(id.clone());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .context("手机连接超时；请检查手机上的 GUI Use 服务状态")??;
    tracing::info!("手机已就绪。首次使用 UI 操作需在手机启用无障碍权限；可以随后关闭设置页面。");
    Ok(Connection {
        device,
        _host: host,
    })
}

async fn broadcast(adb: &Path, serial: &str, command: &str) -> Result<String> {
    adb_command(
        adb,
        Some(serial),
        &[
            "shell",
            "am",
            "broadcast",
            "-n",
            "dev.azurice.guiuse/.DebugControlReceiver",
            "--es",
            "command",
            command,
        ],
        None,
    )
    .await
}

fn broadcast_data(text: &str) -> Option<String> {
    let value = text.split("data=\"").nth(1)?.split('"').next()?;
    Some(value.to_owned())
}

fn needs_control_activity(services: &str) -> bool {
    !(services.contains("ConnectionService") && services.contains("isForeground=true"))
}

fn usb_devices(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let mut columns = line.split_whitespace();
            let serial = columns.next()?;
            if columns.next()? != "device" {
                return None;
            }
            let label = columns
                .find_map(|column| column.strip_prefix("model:"))
                .unwrap_or(serial)
                .replace('_', " ");
            Some((serial.into(), label))
        })
        .collect()
}

async fn adb_command(
    adb: &Path,
    serial: Option<&str>,
    arguments: &[&str],
    input: Option<Vec<u8>>,
) -> Result<String> {
    let operation = async {
        let mut command = Command::new(adb);
        if let Some(serial) = serial {
            command.arg("-s").arg(serial);
        }
        let mut child = command
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("无法启动 adb；请在 nix develop 环境中运行，或配置 GUI_USE_ADB")?;
        if let Some(input) = input {
            child
                .stdin
                .take()
                .context("ADB stdin")?
                .write_all(&input)
                .await?;
        } else {
            drop(child.stdin.take());
        }
        let output = child.wait_with_output().await?;
        ensure!(
            output.status.success(),
            "ADB 操作失败；请检查 USB 调试授权和手机安装状态"
        );
        Ok(String::from_utf8(output.stdout)?)
    };
    tokio::time::timeout(Duration::from_secs(15), operation)
        .await
        .context("ADB 操作超时")?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn usb_selection_ignores_offline_and_unauthorized_devices() {
        let devices = usb_devices(
            "List of devices attached\na unauthorized\nb offline\nc device model:OPPO_Test transport_id:1\n",
        );
        assert_eq!(devices, vec![("c".into(), "OPPO Test".into())]);
        assert!(!needs_control_activity(
            "ConnectionService isForeground=true"
        ));
        assert!(needs_control_activity(
            "ConnectionService isForeground=false"
        ));
        assert!(needs_control_activity(""));
    }
    #[test]
    fn connected_selection_is_unambiguous_and_honors_an_explicit_id() {
        let list = vec![json!({"id":"a"}), json!({"id":"b"})];
        assert!(pick_connected(&list, None).is_err());
        assert_eq!(pick_connected(&list, Some("b")).unwrap(), Some("b".into()));
        assert_eq!(pick_connected(&list, Some("missing")).unwrap(), None);
        assert_eq!(
            broadcast_data("Broadcast completed: result=1, data=\"abc-def\""),
            Some("abc-def".into())
        );
    }

    #[tokio::test]
    async fn existing_host_and_device_are_reused_without_adb_or_identity_files() {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let directory = tempfile::tempdir().unwrap();
        let listener =
            tokio::net::UnixListener::bind(server::control_path(directory.path())).unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..3 {
                let (stream, _) = listener.accept().await.unwrap();
                let (reader, mut writer) = stream.into_split();
                let mut line = String::new();
                BufReader::new(reader).read_line(&mut line).await.unwrap();
                writer
                    .write_all(b"{\"result\":[{\"id\":\"fixture-device\"}]}\n")
                    .await
                    .unwrap();
            }
        });
        let connection = prepare_with_adb(directory.path(), None, None, Path::new("/missing/adb"))
            .await
            .unwrap();
        assert_eq!(connection.device, "fixture-device");
        drop(connection);
        assert_eq!(devices(directory.path()).await.unwrap().len(), 1);
        assert!(!directory.path().join("pairing.json").exists());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn new_identity_is_automatic_and_owned_host_is_cleaned_up_on_setup_failure() {
        let directory = tempfile::tempdir().unwrap();
        let result = prepare_with_adb(
            directory.path(),
            None,
            Some("explicit-phone"),
            Path::new("/missing/adb"),
        )
        .await;
        assert!(result.is_err());
        assert!(identity::load(directory.path()).is_ok());
        tokio::time::timeout(Duration::from_secs(2), async {
            while server::control_path(directory.path()).exists() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
}
