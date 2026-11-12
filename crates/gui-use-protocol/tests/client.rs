use std::sync::{Arc, Mutex};

use futures::executor::block_on;
use gui_use_protocol::*;
use serde_json::{Value, json};

struct Loopback {
    calls: Arc<Mutex<Vec<Value>>>,
    response: Value,
    id_offset: u32,
}

impl RpcTransport for Loopback {
    async fn request(&self, request: RpcRequest) -> RpcResult<RpcResponse> {
        self.calls
            .lock()
            .unwrap()
            .push(serde_json::to_value(&request)?);
        let mut response = self.response.clone();
        response["id"] = json!(request.id + self.id_offset);
        Ok(serde_json::from_value(response)?)
    }
}

fn transport(response: Value) -> (Loopback, Arc<Mutex<Vec<Value>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    (
        Loopback {
            calls: calls.clone(),
            response,
            id_offset: 0,
        },
        calls,
    )
}

#[test]
fn generated_native_method_serializes_jsonrpc_and_decodes_the_typed_result() {
    let (transport, calls) = transport(json!({
        "jsonrpc": "2.0", "id": 1,
        "result": {"apps": [{"package_name": "com.example.app", "label": "Example", "launchable": true}]}
    }));
    let device = DeviceClient::new(transport);
    let result = block_on(device.app_list(Some(false))).unwrap();
    assert_eq!(result.apps[0].label, "Example");
    assert_eq!(
        calls.lock().unwrap()[0],
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "app.list", "params": {"include_system": false}
        })
    );
    block_on(device.app_list(None)).unwrap();
    assert_eq!(calls.lock().unwrap()[1]["id"], 2);
}

#[test]
fn rpc_errors_remain_errors_instead_of_being_deserialized_as_success() {
    let (transport, _) = transport(json!({
        "jsonrpc": "2.0", "id": 1,
        "error": {"code": 2005, "message": "Permission denied", "data": {"permission": "accessibility"}}
    }));
    let error = block_on(DeviceClient::new(transport).app_list(None)).unwrap_err();
    let RpcError::Remote(error) = error else {
        panic!("expected a remote error")
    };
    assert_eq!(error.code, 2005);
    assert_eq!(error.data.unwrap()["permission"], "accessibility");
}

#[test]
fn responses_must_match_the_request_id() {
    let (mut transport, _) = transport(json!({"jsonrpc": "2.0", "id": 1, "result": {"apps": []}}));
    transport.id_offset = 1;
    let result = block_on(DeviceClient::new(transport).app_list(None));
    assert!(matches!(
        result,
        Err(RpcError::UnexpectedId {
            expected: 1,
            actual: 2
        })
    ));
}

#[test]
fn response_envelopes_require_version_and_exactly_one_outcome() {
    for value in [
        json!({"id": 1, "result": null}),
        json!({"jsonrpc": "1.0", "id": 1, "result": null}),
        json!({"jsonrpc": "2.0", "id": 1}),
        json!({"jsonrpc": "2.0", "id": 1, "result": null, "error": {"code": 1, "message": "error"}}),
    ] {
        assert!(serde_json::from_value::<RpcResponse>(value).is_err());
    }
    assert!(matches!(
        serde_json::from_value::<RpcResponse>(json!({
            "jsonrpc": "2.0", "id": 1, "result": null
        }))
        .unwrap(),
        RpcResponse::Success {
            result: Value::Null,
            ..
        }
    ));
}

#[test]
fn signatures_and_doc_comments_are_the_metadata_source() {
    let method = find_method("app.list").unwrap();
    assert_eq!(method.ident, "app_list");
    assert_eq!(method.params, "AppListParams");
    assert_eq!(method.result, "AppListResult");
    assert!(method.docs.contains("默认只返回带 LAUNCHER 入口的包。"));
    assert!(
        method
            .docs
            .contains("```json\n{\"include_system\": false}\n```")
    );
    let (params, result) = (method.schema)();
    let params = serde_json::to_value(params).unwrap();
    assert_eq!(params["$schema"], "http://json-schema.org/draft-07/schema#");
    assert_eq!(
        params["properties"]["include_system"]["description"],
        "是否包含系统应用，默认 false。"
    );
    assert_eq!(
        serde_json::to_value(result).unwrap()["title"],
        "AppListResult"
    );
}

#[test]
fn generic_entry_point_binds_method_params_and_result() {
    let (transport, calls) = transport(json!({
        "jsonrpc": "2.0", "id": 1, "result": {"accepted": true}
    }));
    let client = RpcClient::new(transport);
    let result: AppLaunchResult = block_on(client.call::<AppLaunchRequest>(AppLaunchParams {
        package_name: "com.example.app".into(),
        activity: Some(".MainActivity".into()),
        force_stop: None,
    }))
    .unwrap();
    assert!(result.accepted);
    assert_eq!(
        calls.lock().unwrap()[0]["params"],
        json!({
            "package_name": "com.example.app", "activity": ".MainActivity", "force_stop": null,
        })
    );
    assert_eq!(calls.lock().unwrap()[0]["method"], AppLaunchRequest::METHOD);
}

mod sample {
    use gui_use_protocol::rpc_service;

    #[rpc_service]
    pub trait Sample {
        #[rpc(name = "ping")]
        async fn ping();

        #[rpc(name = "sum")]
        async fn sum(a: i32, b: i32) -> i32;

        #[rpc(name = "names")]
        async fn names(prefix: Option<String>) -> Vec<String>;
    }
}

#[test]
fn generated_clients_support_empty_multi_argument_and_container_methods() {
    let (wire, calls) = transport(json!({"jsonrpc": "2.0", "id": 1, "result": 5}));
    let client = sample::SampleClient::new(wire);
    assert_eq!(block_on(client.sum(2, 3)).unwrap(), 5);
    assert_eq!(calls.lock().unwrap()[0]["params"], json!({"a": 2, "b": 3}));
    assert_eq!(block_on(sample::Sample::sum(&client, 3, 2)).unwrap(), 5);
    let (wire, calls) = transport(json!({"jsonrpc": "2.0", "id": 1, "result": null}));
    let client = sample::SampleClient::new(wire);
    block_on(client.ping()).unwrap();
    block_on(sample::Sample::ping(&client)).unwrap();
    assert_eq!(calls.lock().unwrap()[0]["params"], json!({}));
    let (wire, _) = transport(json!({"jsonrpc": "2.0", "id": 1, "result": ["Alice"]}));
    let client = sample::SampleClient::new(wire);
    assert_eq!(
        block_on(sample::Sample::names(&client, None)).unwrap(),
        ["Alice"]
    );
}

#[test]
fn typed_events_keep_the_notification_envelope_and_reject_wrong_event_types() {
    let notification = RpcNotification::event::<NotificationRemoved>(
        43,
        "2026-11-12T01:23:45.678Z".into(),
        NotificationRemovedData {
            key: "notification-key".into(),
        },
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&notification).unwrap(),
        json!({
            "jsonrpc": "2.0", "method": "events", "params": {
                "seq": 43, "ts": "2026-11-12T01:23:45.678Z",
                "type": "notification.removed", "data": {"key": "notification-key"},
            }
        })
    );
    let decoded = notification.decode_event::<NotificationRemoved>().unwrap();
    assert_eq!(decoded.data.key, "notification-key");
    assert_eq!(decoded.seq, 43);
    assert!(matches!(
        notification.decode_event::<NotificationPosted>(),
        Err(RpcError::UnexpectedEvent { .. })
    ));
    let mut wrong_method = notification;
    wrong_method.method = "app.list".into();
    assert!(matches!(
        wrong_method.decode_event::<NotificationRemoved>(),
        Err(RpcError::UnexpectedEvent { .. })
    ));
    let mut request = serde_json::to_value(wrong_method).unwrap();
    request["id"] = json!(1);
    assert!(serde_json::from_value::<RpcNotification>(request).is_err());
}
