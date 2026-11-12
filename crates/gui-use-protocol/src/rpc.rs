//! JSON-RPC envelopes and a transport-independent typed client.

use std::sync::atomic::{AtomicU32, Ordering};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

pub type RpcResult<T> = Result<T, RpcError>;

/// A method identity binds the wire name, parameters and result together.
///
/// ```compile_fail
/// use gui_use_protocol::{AppListRequest, AppLaunchParams, RpcClient, RpcTransport};
/// fn wrong_parameters<T: RpcTransport>(client: &RpcClient<T>, params: AppLaunchParams) {
///     client.call::<AppListRequest>(params);
/// }
/// ```
pub trait RpcMethod {
    const METHOD: &'static str;
    type Params: Serialize + DeserializeOwned;
    type Result: Serialize + DeserializeOwned;
}

/// Events have a payload, but no request ID or result type.
pub trait RpcEvent {
    const NAME: &'static str;
    type Data: Serialize + DeserializeOwned;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EventParams<D> {
    pub seq: i32,
    pub ts: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub data: D,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RpcNotification {
    pub jsonrpc: JsonRpcVersion,
    pub method: String,
    pub params: Value,
}

impl RpcNotification {
    pub fn event<E: RpcEvent>(seq: i32, ts: String, data: E::Data) -> RpcResult<Self> {
        Ok(Self {
            jsonrpc: JsonRpcVersion::V2,
            method: "events".into(),
            params: serde_json::to_value(EventParams {
                seq,
                ts,
                event_type: E::NAME.to_string(),
                data,
            })?,
        })
    }

    pub fn decode_event<E: RpcEvent>(&self) -> RpcResult<EventParams<E::Data>> {
        if self.method != "events" {
            return Err(RpcError::UnexpectedEvent {
                expected: "events",
                actual: self.method.clone(),
            });
        }
        let event_type = self
            .params
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("");
        if event_type != E::NAME {
            return Err(RpcError::UnexpectedEvent {
                expected: E::NAME,
                actual: event_type.to_string(),
            });
        }
        Ok(serde_json::from_value(self.params.clone())?)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum JsonRpcVersion {
    #[serde(rename = "2.0")]
    V2,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcRequest {
    pub jsonrpc: JsonRpcVersion,
    pub id: u32,
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcErrorObject {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum RpcResponse {
    Success {
        jsonrpc: JsonRpcVersion,
        id: u32,
        result: Value,
    },
    Failure {
        jsonrpc: JsonRpcVersion,
        id: u32,
        error: RpcErrorObject,
    },
}

#[derive(Debug)]
pub enum RpcError {
    Json(serde_json::Error),
    Remote(RpcErrorObject),
    UnexpectedId {
        expected: u32,
        actual: u32,
    },
    UnexpectedEvent {
        expected: &'static str,
        actual: String,
    },
    IdExhausted,
    Transport(String),
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(error) => write!(f, "invalid RPC JSON: {error}"),
            Self::Remote(error) => write!(f, "RPC error {}: {}", error.code, error.message),
            Self::UnexpectedId { expected, actual } => {
                write!(f, "expected RPC id {expected}, got {actual}")
            }
            Self::UnexpectedEvent { expected, actual } => {
                write!(f, "expected event {expected}, got {actual}")
            }
            Self::IdExhausted => write!(f, "RPC request IDs exhausted; create a new session"),
            Self::Transport(error) => write!(f, "RPC transport error: {error}"),
        }
    }
}

impl std::error::Error for RpcError {}

impl From<serde_json::Error> for RpcError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

/// Routes an envelope to its peer and correlates its response over the connection.
/// WebSocket connection management remains the host's responsibility.
#[allow(async_fn_in_trait)]
pub trait RpcTransport {
    async fn request(&self, request: RpcRequest) -> RpcResult<RpcResponse>;
}

pub struct RpcClient<T> {
    transport: T,
    next_id: AtomicU32,
}

impl<T: RpcTransport> RpcClient<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            next_id: AtomicU32::new(1),
        }
    }

    pub async fn call<M: RpcMethod>(&self, params: M::Params) -> RpcResult<M::Result> {
        let params = serde_json::to_value(params)?;
        let id = self
            .next_id
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| RpcError::IdExhausted)?;
        let response = self
            .transport
            .request(RpcRequest {
                jsonrpc: JsonRpcVersion::V2,
                id,
                method: M::METHOD.to_string(),
                params,
            })
            .await?;
        let actual = match &response {
            RpcResponse::Success { id, .. } | RpcResponse::Failure { id, .. } => *id,
        };
        if actual != id {
            return Err(RpcError::UnexpectedId {
                expected: id,
                actual,
            });
        }
        match response {
            RpcResponse::Success { result, .. } => Ok(serde_json::from_value(result)?),
            RpcResponse::Failure { error, .. } => Err(RpcError::Remote(error)),
        }
    }
}
