use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::net::SocketAddr;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use gui_use_protocol::{
    EVENTS, EventParams, HelloParams, HelloResult, JsonRpcVersion, METHODS, RpcNotification,
    RpcRequest, RpcResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, UnixListener, UnixStream};
use tokio::sync::{RwLock, Semaphore, mpsc};
use tokio::task::JoinSet;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::{
    WebSocketStream, accept_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

use crate::{
    identity,
    transport::{WsTransport, binary_frame},
};

struct Device {
    session: u64,
    hello: HelloParams,
    transport: WsTransport,
}

#[derive(Default)]
struct State {
    next_session: AtomicU64,
    devices: RwLock<BTreeMap<String, Arc<Device>>>,
    events: Mutex<VecDeque<Value>>,
}

#[derive(Serialize, Deserialize)]
pub struct ControlRequest {
    pub method: String,
    #[serde(default)]
    pub params: Value,
    pub device: Option<String>,
    pub timeout_ms: Option<u64>,
}

struct SocketPath(PathBuf);
impl Drop for SocketPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub fn control_path(directory: &Path) -> PathBuf {
    directory.join("control.sock")
}

pub async fn run(directory: &Path, listen: SocketAddr) -> Result<()> {
    let (pairing, tls) = identity::load(directory)?;
    let acceptor = TlsAcceptor::from(tls);
    let tcp = TcpListener::bind(listen).await?;
    let path = control_path(directory);
    if path.exists() {
        ensure!(
            std::fs::symlink_metadata(&path)?.file_type().is_socket(),
            "Control path is not a socket"
        );
        if UnixStream::connect(&path).await.is_ok() {
            bail!("Another host is already running")
        }
        std::fs::remove_file(&path)?;
    }
    let control = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    let _cleanup = SocketPath(path);
    let token = Arc::new(pairing.token);
    let state = Arc::new(State::default());
    let mut tasks = JoinSet::new();
    let connection_limit = Arc::new(Semaphore::new(32));
    tracing::info!(address = %listen, "Listening for paired devices");
    loop {
        tokio::select! {
            incoming = tcp.accept() => {
                let (stream, _) = incoming?;
                let Ok(permit) = connection_limit.clone().try_acquire_owned() else { continue };
                let acceptor = acceptor.clone(); let token = token.clone(); let state = state.clone();
                tasks.spawn(async move {
                    let _permit = permit;
                    let result: Result<()> = async {
                        let tls = tokio::time::timeout(Duration::from_secs(10), acceptor.accept(stream)).await??;
                        let mut config = WebSocketConfig::default();
                        config.max_message_size = Some(16 * 1024 * 1024); config.max_frame_size = Some(16 * 1024 * 1024);
                        let socket = tokio::time::timeout(Duration::from_secs(10), accept_async_with_config(tls, Some(config))).await??;
                        connection(socket, &token, state).await
                    }.await;
                    if result.is_err() { tracing::warn!("Device connection closed or rejected"); }
                });
            }
            incoming = control.accept() => {
                let (stream, _) = incoming?; let state = state.clone();
                tasks.spawn(async move { let _ = handle_control(stream, state).await; });
            }
            _ = tasks.join_next(), if !tasks.is_empty() => {}
        }
    }
}

fn hello(request: &RpcRequest, token: &str) -> Result<HelloParams> {
    ensure!(
        request.method == "hello" && request.id == 0,
        "Expected hello"
    );
    let hello: HelloParams = serde_json::from_value(request.params.clone())?;
    ensure!(hello.protocol == 1, "Unsupported protocol");
    ensure!(
        bool::from(hello.token.as_bytes().ct_eq(token.as_bytes())),
        "Authentication failed"
    );
    ensure!(
        !hello.device.id.is_empty()
            && hello.device.id.len() <= 128
            && hello.device.model.len() <= 128,
        "Invalid device identity"
    );
    let known: BTreeSet<_> = METHODS.iter().map(|method| method.name).collect();
    ensure!(
        hello
            .capabilities
            .iter()
            .all(|name| known.contains(name.as_str())),
        "Unknown capabilities"
    );
    Ok(hello)
}

async fn connection<S>(mut socket: WebSocketStream<S>, token: &str, state: Arc<State>) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let first = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await?
        .context("Disconnected before hello")??;
    let Message::Text(text) = first else {
        bail!("Expected JSON hello")
    };
    ensure!(text.len() <= 65536, "Hello is too large");
    let request: RpcRequest = serde_json::from_str(&text)?;
    let mut hello = match hello(&request, token) {
        Ok(hello) => hello,
        Err(error) => {
            let _ = socket.close(None).await;
            return Err(error);
        }
    };
    // Credentials are never retained in device listings or logs.
    hello.token.clear();
    socket
        .send(Message::Text(
            serde_json::to_string(&RpcResponse::Success {
                jsonrpc: JsonRpcVersion::V2,
                id: 0,
                result: serde_json::to_value(HelloResult {
                    protocol: 1,
                    events: EVENTS.iter().map(|event| event.name.into()).collect(),
                })?,
            })?
            .into(),
        ))
        .await?;
    let (tx, mut outgoing) = mpsc::channel(64);
    let device = Arc::new(Device {
        session: state.next_session.fetch_add(1, Ordering::Relaxed) + 1,
        hello,
        transport: WsTransport::new(tx),
    });
    let id = device.hello.device.id.clone();
    if let Some(previous) = state
        .devices
        .write()
        .await
        .insert(id.clone(), device.clone())
    {
        let _ = previous.transport.0.tx.try_send(Message::Close(None));
    }
    tracing::info!(model = %device.hello.device.model, sdk = device.hello.device.sdk, "Device connected");
    let mut sequence = 0;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
    let mut last_frame = Instant::now();
    let outcome = async {
        loop {
            tokio::select! {
                incoming = socket.next() => {
                    let Some(message) = incoming else { break };
                    last_frame = Instant::now();
                    match message? {
                        Message::Text(text) => {
                            if let Ok(response) = serde_json::from_str::<RpcResponse>(&text) {
                                let id = match &response { RpcResponse::Success { id, .. } | RpcResponse::Failure { id, .. } => *id };
                                if let Some(sender) = device.transport.0.responses.lock().unwrap().remove(&id) { let _ = sender.send(response); }
                            } else {
                                let notification: RpcNotification = serde_json::from_str(&text)?;
                                ensure!(notification.method == "events", "Unknown notification method");
                                let params: EventParams<Value> = serde_json::from_value(notification.params.clone())?;
                                ensure!(params.seq > sequence && EVENTS.iter().any(|event| event.name == params.event_type), "Invalid event sequence or type");
                                let gap = params.seq != sequence + 1;
                                sequence = params.seq;
                                let mut events = state.events.lock().unwrap();
                                if events.len() == 512 { events.pop_front(); }
                                events.push_back(json!({"device": id, "session": device.session, "event": notification, "reconcile_required": gap}));
                            }
                        }
                        Message::Binary(bytes) => {
                            let (id, bytes) = binary_frame(&bytes)?;
                            if let Some(sender) = device.transport.0.binaries.lock().unwrap().remove(&id) { let _ = sender.send(bytes); }
                        }
                        Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await?,
                        Message::Close(_) => break,
                        _ => {}
                    }
                }
                outgoing = outgoing.recv() => {
                    let Some(message) = outgoing else { break };
                    let closing = matches!(message, Message::Close(_));
                    socket.send(message).await?;
                    if closing { break }
                }
                _ = heartbeat.tick() => {
                    ensure!(last_frame.elapsed() < Duration::from_secs(30), "Heartbeat timed out");
                    socket.send(Message::Ping(Vec::new().into())).await?;
                }
            }
        }
        Ok(())
    }.await;
    let mut devices = state.devices.write().await;
    if devices
        .get(&id)
        .is_some_and(|current| Arc::ptr_eq(current, &device))
    {
        devices.remove(&id);
    }
    device.transport.0.responses.lock().unwrap().clear();
    device.transport.0.binaries.lock().unwrap().clear();
    outcome
}

async fn execute(request: ControlRequest, state: Arc<State>) -> Result<Value> {
    if request.method == "host.devices" {
        let devices = state.devices.read().await;
        return Ok(
            json!({"result": devices.values().map(|device| json!({"id":device.hello.device.id,"session":device.session,"model":device.hello.device.model,"sdk":device.hello.device.sdk,"capabilities":device.hello.capabilities})).collect::<Vec<_>>()}),
        );
    }
    if request.method == "host.events" {
        let events = state.events.lock().unwrap();
        return Ok(
            json!({"result":events.iter().filter(|event| request.device.as_ref().is_none_or(|id| event["device"] == *id)).cloned().collect::<Vec<_>>()}),
        );
    }
    let timeout = request.timeout_ms.unwrap_or(10000);
    ensure!((1..=120000).contains(&timeout), "Invalid timeout");
    ensure!(request.params.is_object(), "Parameters must be an object");
    let device = {
        let devices = state.devices.read().await;
        if let Some(id) = request.device {
            devices
                .get(&id)
                .cloned()
                .context("Device is not connected")?
        } else {
            ensure!(
                devices.len() == 1,
                "Select --device when zero or multiple devices are connected"
            );
            devices.values().next().unwrap().clone()
        }
    };
    if !device.hello.capabilities.contains(&request.method) {
        return Ok(
            json!({"error":{"code":-32601,"message":"Method is not supported by this device"}}),
        );
    }
    let (response, binary) = device
        .transport
        .exchange(
            &request.method,
            request.params,
            Duration::from_millis(timeout),
        )
        .await?;
    Ok(match response {
        RpcResponse::Success { result, .. } => {
            let mut result = json!({"result":result});
            if let Some(bytes) = binary {
                result["binary"] = json!(STANDARD.encode(bytes));
            }
            result
        }
        RpcResponse::Failure { error, .. } => json!({"error":error}),
    })
}

async fn handle_control(stream: UnixStream, state: Arc<State>) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut line = String::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        BufReader::new(reader).read_line(&mut line),
    )
    .await??;
    ensure!(line.len() <= 1024 * 1024, "Control request too large");
    let response = match serde_json::from_str::<ControlRequest>(&line) {
        Ok(request) => match execute(request, state).await {
            Ok(value) => value,
            Err(error) => json!({"error":{"code":-32000,"message":error.to_string()}}),
        },
        Err(_) => json!({"error":{"code":-32700,"message":"Invalid control request"}}),
    };
    writer
        .write_all(serde_json::to_string(&response)?.as_bytes())
        .await?;
    writer.write_all(b"\n").await?;
    Ok(())
}

pub async fn control(directory: &Path, request: ControlRequest) -> Result<Value> {
    let stream = UnixStream::connect(control_path(directory))
        .await
        .context("Host is not running")?;
    let (reader, mut writer) = stream.into_split();
    writer
        .write_all(serde_json::to_string(&request)?.as_bytes())
        .await?;
    writer.write_all(b"\n").await?;
    let mut line = String::new();
    tokio::time::timeout(
        Duration::from_secs(125),
        BufReader::new(reader).read_line(&mut line),
    )
    .await??;
    Ok(serde_json::from_str(&line)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn handshake_requires_token_protocol_and_known_capabilities() {
        let mut request = RpcRequest {
            jsonrpc: JsonRpcVersion::V2,
            id: 0,
            method: "hello".into(),
            params: json!({"protocol":1,"token":"secret","device":{"id":"test","model":"local","sdk":36},"capabilities":["app.list"]}),
        };
        assert!(hello(&request, "secret").is_ok());
        assert!(hello(&request, "wrong").is_err());
        request.params["protocol"] = json!(2);
        assert!(hello(&request, "secret").is_err());
        request.params["protocol"] = json!(1);
        request.params["capabilities"] = json!(["unknown"]);
        assert!(hello(&request, "secret").is_err());
    }

    #[tokio::test]
    async fn websocket_handshake_requests_events_and_disconnect_work_together() {
        use tokio_tungstenite::tungstenite::protocol::Role;
        let state = Arc::new(State::default());
        let (a, b) = tokio::io::duplex(65536);
        let server_socket = WebSocketStream::from_raw_socket(a, Role::Server, None).await;
        let mut phone = WebSocketStream::from_raw_socket(b, Role::Client, None).await;
        let server_state = state.clone();
        let server =
            tokio::spawn(async move { connection(server_socket, "secret", server_state).await });
        phone.send(Message::Text(json!({"jsonrpc":"2.0","id":0,"method":"hello","params":{"protocol":1,"token":"secret","device":{"id":"test","model":"fixture","sdk":36},"capabilities":["app.list"]}}).to_string().into())).await.unwrap();
        let Message::Text(hello) = phone.next().await.unwrap().unwrap() else {
            panic!("hello response")
        };
        assert_eq!(
            serde_json::from_str::<Value>(&hello).unwrap()["result"]["protocol"],
            1
        );
        let caller_state = state.clone();
        let caller = tokio::spawn(async move {
            execute(
                ControlRequest {
                    method: "app.list".into(),
                    params: json!({}),
                    device: None,
                    timeout_ms: Some(1000),
                },
                caller_state,
            )
            .await
        });
        let request = loop {
            match phone.next().await.unwrap().unwrap() {
                Message::Text(text) => break serde_json::from_str::<RpcRequest>(&text).unwrap(),
                Message::Ping(data) => phone.send(Message::Pong(data)).await.unwrap(),
                _ => {}
            }
        };
        phone
            .send(Message::Text(
                json!({"jsonrpc":"2.0","id":request.id,"result":{"apps":[]}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        assert_eq!(caller.await.unwrap().unwrap()["result"], json!({"apps":[]}));
        phone.send(Message::Text(json!({"jsonrpc":"2.0","method":"events","params":{"seq":2,"ts":"2026-11-12T00:00:00Z","type":"notification.removed","data":{"key":"fixture"}}}).to_string().into())).await.unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while state.events.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(state.events.lock().unwrap()[0]["reconcile_required"], true);
        phone.close(None).await.unwrap();
        server.await.unwrap().unwrap();
        assert!(state.devices.read().await.is_empty());
    }
}
