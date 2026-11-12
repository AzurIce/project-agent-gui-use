//! Use the generated Rust client against an already-running local host.

use std::path::PathBuf;

use gui_use_protocol::{
    JsonRpcVersion, RpcError, RpcRequest, RpcResponse, RpcResult, RpcTransport,
};

use crate::server::{self, ControlRequest};

#[derive(Clone)]
pub struct LocalTransport {
    pub directory: PathBuf,
    pub device: Option<String>,
    pub timeout_ms: u64,
}

impl LocalTransport {
    pub fn new(directory: impl Into<PathBuf>, device: Option<String>) -> Self {
        Self {
            directory: directory.into(),
            device,
            timeout_ms: 70000,
        }
    }
}

impl RpcTransport for LocalTransport {
    async fn request(&self, request: RpcRequest) -> RpcResult<RpcResponse> {
        let response = server::control(
            &self.directory,
            ControlRequest {
                method: request.method,
                params: request.params,
                device: self.device.clone(),
                timeout_ms: Some(self.timeout_ms),
            },
        )
        .await
        .map_err(|error| RpcError::Transport(error.to_string()))?;
        if let Some(error) = response.get("error") {
            Ok(RpcResponse::Failure {
                jsonrpc: JsonRpcVersion::V2,
                id: request.id,
                error: serde_json::from_value(error.clone())?,
            })
        } else {
            Ok(RpcResponse::Success {
                jsonrpc: JsonRpcVersion::V2,
                id: request.id,
                result: response
                    .get("result")
                    .cloned()
                    .ok_or_else(|| RpcError::Transport("Host response has no result".into()))?,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gui_use_protocol::DeviceClient;
    use serde_json::json;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    #[tokio::test]
    async fn native_calls_reach_the_local_host_with_named_parameters() {
        let directory = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(server::control_path(directory.path())).unwrap();
        let responder = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let mut line = String::new();
            BufReader::new(reader).read_line(&mut line).await.unwrap();
            let request: ControlRequest = serde_json::from_str(&line).unwrap();
            assert_eq!(request.method, "app.launch");
            assert_eq!(
                request.params,
                json!({"package_name":"local.test","activity":null,"force_stop":null})
            );
            writer
                .write_all(b"{\"result\":{\"accepted\":true}}\n")
                .await
                .unwrap();
        });
        let client = DeviceClient::new(LocalTransport::new(directory.path(), None));
        let result =
            tokio::spawn(async move { client.app_launch("local.test".into(), None, None).await })
                .await
                .unwrap()
                .unwrap();
        assert!(result.accepted);
        responder.await.unwrap();
    }
}
