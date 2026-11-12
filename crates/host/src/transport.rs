use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU32, Ordering},
};
use std::time::Duration;

use gui_use_protocol::{
    JsonRpcVersion, RpcError, RpcRequest, RpcResponse, RpcResult, RpcTransport,
};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

pub struct Inner {
    pub tx: mpsc::Sender<Message>,
    pub responses: Mutex<HashMap<u32, oneshot::Sender<RpcResponse>>>,
    pub binaries: Mutex<HashMap<u32, oneshot::Sender<Vec<u8>>>>,
    next_id: AtomicU32,
}

#[derive(Clone)]
pub struct WsTransport(pub Arc<Inner>);

struct Pending {
    inner: Arc<Inner>,
    id: u32,
}
impl Drop for Pending {
    fn drop(&mut self) {
        let pending = self
            .inner
            .responses
            .lock()
            .unwrap()
            .remove(&self.id)
            .is_some();
        self.inner.binaries.lock().unwrap().remove(&self.id);
        if pending {
            let cancel =
                serde_json::json!({"jsonrpc":"2.0","method":"$/cancel","params":{"id":self.id}});
            let _ = self
                .inner
                .tx
                .try_send(Message::Text(cancel.to_string().into()));
        }
    }
}

impl WsTransport {
    pub fn new(tx: mpsc::Sender<Message>) -> Self {
        Self(Arc::new(Inner {
            tx,
            responses: Mutex::new(HashMap::new()),
            binaries: Mutex::new(HashMap::new()),
            next_id: AtomicU32::new(1),
        }))
    }

    pub async fn exchange(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> RpcResult<(RpcResponse, Option<Vec<u8>>)> {
        let id = self
            .0
            .next_id
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| RpcError::IdExhausted)?;
        let (response_tx, response_rx) = oneshot::channel();
        self.0.responses.lock().unwrap().insert(id, response_tx);
        let binary = if method == "ui.screenshot" {
            let (tx, rx) = oneshot::channel();
            self.0.binaries.lock().unwrap().insert(id, tx);
            Some(rx)
        } else {
            None
        };
        let _pending = Pending {
            inner: self.0.clone(),
            id,
        };
        tokio::time::timeout(timeout, async {
            let request = RpcRequest {
                jsonrpc: JsonRpcVersion::V2,
                id,
                method: method.into(),
                params,
            };
            self.0
                .tx
                .send(Message::Text(serde_json::to_string(&request)?.into()))
                .await
                .map_err(|_| RpcError::Transport("Device disconnected".into()))?;
            let response = response_rx
                .await
                .map_err(|_| RpcError::Transport("Device disconnected".into()))?;
            let bytes = if let Some(rx) = binary
                && matches!(response, RpcResponse::Success { .. })
            {
                let bytes = rx
                    .await
                    .map_err(|_| RpcError::Transport("Missing screenshot frame".into()))?;
                if let RpcResponse::Success { result, .. } = &response
                    && (result["bytes"].as_u64() != Some(bytes.len() as u64)
                        || !bytes.starts_with(b"\x89PNG\r\n\x1a\n"))
                {
                    return Err(RpcError::Transport("Invalid screenshot payload".into()));
                }
                Some(bytes)
            } else {
                None
            };
            Ok((response, bytes))
        })
        .await
        .map_err(|_| RpcError::Transport("Request timed out".into()))?
    }
}

impl RpcTransport for WsTransport {
    async fn request(&self, request: RpcRequest) -> RpcResult<RpcResponse> {
        let (mut response, _) = self
            .exchange(&request.method, request.params, Duration::from_secs(70))
            .await?;
        // Each client may allocate its own IDs; the connection owns the actual wire IDs.
        match &mut response {
            RpcResponse::Success { id, .. } | RpcResponse::Failure { id, .. } => *id = request.id,
        }
        Ok(response)
    }
}

pub fn binary_frame(bytes: &[u8]) -> anyhow::Result<(u32, Vec<u8>)> {
    anyhow::ensure!(
        bytes.len() >= 4 && bytes.len() <= 16 * 1024 * 1024,
        "Invalid binary frame size"
    );
    let length = u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
    anyhow::ensure!(
        (1..=65536).contains(&length) && length <= bytes.len() - 4,
        "Invalid binary header length"
    );
    let header: Value = serde_json::from_slice(&bytes[4..4 + length])?;
    let id = header["id"]
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .ok_or_else(|| anyhow::anyhow!("Missing binary request ID"))?;
    anyhow::ensure!(
        header["mime"] == "image/png",
        "Unsupported binary MIME type"
    );
    Ok((id, bytes[4 + length..].to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gui_use_protocol::{AppListParams, AppListRequest, RpcClient};
    use serde_json::json;

    #[tokio::test]
    async fn generated_clients_share_a_connection_without_id_collisions() {
        let (tx, mut rx) = mpsc::channel(4);
        let transport = WsTransport::new(tx);
        let client_a = RpcClient::new(transport.clone());
        let client_b = RpcClient::new(transport.clone());
        let responder = tokio::spawn(async move {
            let mut ids = Vec::new();
            for _ in 0..2 {
                let Message::Text(text) = rx.recv().await.unwrap() else {
                    panic!()
                };
                let request: RpcRequest = serde_json::from_str(&text).unwrap();
                ids.push(request.id);
                transport
                    .0
                    .responses
                    .lock()
                    .unwrap()
                    .remove(&request.id)
                    .unwrap()
                    .send(RpcResponse::Success {
                        jsonrpc: JsonRpcVersion::V2,
                        id: request.id,
                        result: json!({"apps": []}),
                    })
                    .unwrap();
            }
            assert_ne!(ids[0], ids[1]);
        });
        let (a, b) = tokio::join!(
            client_a.call::<AppListRequest>(AppListParams {
                include_system: None
            }),
            client_b.call::<AppListRequest>(AppListParams {
                include_system: None
            })
        );
        assert!(a.unwrap().apps.is_empty());
        assert!(b.unwrap().apps.is_empty());
        responder.await.unwrap();
    }

    #[tokio::test]
    async fn timed_out_requests_leave_no_pending_entries() {
        let (tx, _rx) = mpsc::channel(4);
        let transport = WsTransport::new(tx);
        assert!(
            transport
                .exchange("app.list", json!({}), Duration::from_millis(5))
                .await
                .is_err()
        );
        assert!(transport.0.responses.lock().unwrap().is_empty());
        assert!(transport.0.binaries.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancelling_a_caller_removes_pending_state_and_notifies_the_device() {
        let (tx, mut rx) = mpsc::channel(4);
        let transport = WsTransport::new(tx);
        let caller_transport = transport.clone();
        let caller = tokio::spawn(async move {
            caller_transport
                .exchange("ui.wait", json!({"until":"idle"}), Duration::from_secs(30))
                .await
        });
        let Message::Text(request) = rx.recv().await.unwrap() else {
            panic!()
        };
        let request: RpcRequest = serde_json::from_str(&request).unwrap();
        caller.abort();
        let _ = caller.await;
        let Message::Text(cancel) = rx.recv().await.unwrap() else {
            panic!()
        };
        let cancel: Value = serde_json::from_str(&cancel).unwrap();
        assert_eq!(cancel["method"], "$/cancel");
        assert_eq!(cancel["params"]["id"], request.id);
        assert!(transport.0.responses.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn screenshots_can_arrive_before_their_json_response() {
        let (tx, mut rx) = mpsc::channel(4);
        let transport = WsTransport::new(tx);
        let responder_transport = transport.clone();
        let responder = tokio::spawn(async move {
            let Message::Text(text) = rx.recv().await.unwrap() else {
                panic!()
            };
            let request: RpcRequest = serde_json::from_str(&text).unwrap();
            let bytes = b"\x89PNG\r\n\x1a\nfixture".to_vec();
            responder_transport
                .0
                .binaries
                .lock()
                .unwrap()
                .remove(&request.id)
                .unwrap()
                .send(bytes.clone())
                .unwrap();
            responder_transport
                .0
                .responses
                .lock()
                .unwrap()
                .remove(&request.id)
                .unwrap()
                .send(RpcResponse::Success {
                    jsonrpc: JsonRpcVersion::V2,
                    id: request.id,
                    result: json!({"bytes":bytes.len()}),
                })
                .unwrap();
        });
        let (_, bytes) = transport
            .exchange("ui.screenshot", json!({}), Duration::from_secs(1))
            .await
            .unwrap();
        assert!(bytes.unwrap().ends_with(b"fixture"));
        responder.await.unwrap();
    }

    #[test]
    fn binary_headers_are_bounded_and_correlated() {
        for invalid in [&[][..], &[0, 0, 0, 0], &[255, 255, 255, 255]] {
            assert!(binary_frame(invalid).is_err());
        }
        let header = br#"{"id":9,"mime":"image/png"}"#;
        let mut bytes = (header.len() as u32).to_be_bytes().to_vec();
        bytes.extend(header);
        bytes.extend(b"PNG");
        assert_eq!(binary_frame(&bytes).unwrap(), (9, b"PNG".to_vec()));
    }
}
