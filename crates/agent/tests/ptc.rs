use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use gui_use_agent::{
    agent::{self, Limits},
    executor::{Config, Executor, read_frame},
    model::Model,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, UnixListener};
use tracing::instrument::WithSubscriber;

#[derive(Clone, Default)]
struct LogBuffer(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl LogBuffer {
    fn events(&self) -> Vec<Value> {
        String::from_utf8(self.0.lock().unwrap().clone())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync + 'static {
        let buffer = self.clone();
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .with_ansi(false)
            .with_writer(move || buffer.clone())
            .json()
            .finish()
    }
}

fn executor(directory: &std::path::Path, actions: bool) -> Executor {
    Executor::new(Config {
        worker: env!("CARGO_BIN_EXE_gui-use-agent").into(),
        directory: directory.into(),
        device: "fixture-device".into(),
        artifacts: directory.join("artifacts"),
        allow_actions: actions,
        allow_notification_reply: false,
        timeout_ms: 3000,
    })
}

struct Host {
    task: tokio::task::JoinHandle<()>,
    calls: Arc<AtomicUsize>,
    count: Arc<AtomicUsize>,
}

impl Drop for Host {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn host(directory: &std::path::Path) -> Host {
    let listener = UnixListener::bind(directory.join("control.sock")).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let counter = count.clone();
    let task = tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let (reader, mut writer) = stream.into_split();
            let request: Value = serde_json::from_slice(
                &read_frame(&mut BufReader::new(reader), 65536)
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(request["device"], "fixture-device");
            seen.fetch_add(1, Ordering::Relaxed);
            let response = match request["method"].as_str().unwrap() {
                "ui.snapshot" => {
                    let mut nodes = (0..1000).map(|i| json!({"id":format!("background-{i}"),"text":format!("PRIVATE_UNSELECTED_ROW_{i}"),"actions":[]})).collect::<Vec<_>>();
                    nodes.push(json!({"id":"increment","text":"Increment","actions":["click"]}));
                    nodes.push(json!({"id":"counter","text":format!("Count: {}",counter.load(Ordering::Relaxed)),"actions":[]}));
                    json!({"result":{"package_name":"dev.azurice.guiuse","nodes":nodes,"truncated":false}})
                }
                "ui.act" => {
                    counter.fetch_add(1, Ordering::Relaxed);
                    json!({"result":{"changed":true}})
                }
                "app.list" => json!({"result":{"apps":[]}}),
                "ui.screenshot" => {
                    json!({"result":{"mime":"image/png","width":1,"height":1,"bytes":11},"binary":"iVBORw0KGgpQTkc="})
                }
                "ui.wait" => {
                    if request["params"]["value"] == "blocked" {
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                    json!({"result":{"matched":true}})
                }
                _ => panic!("Unexpected method: {}", request["method"]),
            };
            let _ = writer.write_all(format!("{response}\n").as_bytes()).await;
        }
    });
    Host { task, calls, count }
}

async fn scripted_model(replies: Vec<Value>) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for reply in replies {
            let (stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(stream);
            let mut header = String::new();
            reader.read_line(&mut header).await.unwrap();
            assert!(header.starts_with("POST /v1/chat/completions "));
            let mut length = 0;
            loop {
                header.clear();
                reader.read_line(&mut header).await.unwrap();
                if header == "\r\n" {
                    break;
                }
                if let Some(value) = header.to_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
            }
            assert!(length <= 1024 * 1024);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).await.unwrap();
            requests.push(serde_json::from_slice(&body).unwrap());
            let body = reply.to_string();
            reader.get_mut().write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    (format!("http://{address}/v1"), task)
}

fn tool(id: &str, code: &str) -> Value {
    json!({"choices":[{"finish_reason":"tool_calls","message":{"role":"assistant","content":null,"tool_calls":[{"id":id,"type":"function","function":{"name":"android_code","arguments":json!({"code":code}).to_string()}}]}}],
        "usage":{"prompt_tokens":100,"completion_tokens":20}})
}

fn done(text: &str) -> Value {
    json!({"choices":[{"finish_reason":"stop","message":{"role":"assistant","content":text}}]})
}

#[tokio::test]
async fn model_code_host_loop_filters_large_trees_and_verifies_actions() {
    let directory = tempfile::tempdir().unwrap();
    let host = host(directory.path()).await;
    let mut executor = executor(directory.path(), true);
    let (base, server) = scripted_model(vec![
        tool("inspect", "const s=await android.ui.snapshot({filter:'all'}); state.before=Number(s.nodes.find(n=>n.text?.startsWith('Count:')).text.split(':')[1]); return {before:state.before};"),
        tool("act", "const s=await android.ui.snapshot({filter:'all'}); const n=s.nodes.find(n=>n.text==='Increment'); await android.ui.act({node:n.id,action:'click'}); await android.ui.wait({until:'idle'}); const after=await android.ui.snapshot({filter:'all'}); return {before:state.before,after:after.nodes.find(n=>n.text?.startsWith('Count:')).text};"),
        done("Verified Count: 1"),
    ]).await;
    let model = Model::new(&base, "fixture-model".into(), None).unwrap();
    let outcome = agent::run(
        &model,
        &mut executor,
        "Click Increment once and verify",
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(outcome.code_executions, 2);
    assert_eq!(outcome.rpc_calls, 5);
    assert_eq!(outcome.answer, "Verified Count: 1");
    assert_eq!(host.count.load(Ordering::Relaxed), 1);
    assert!(outcome.rpc_response_bytes > 100 * outcome.tool_result_bytes);
    let requests = server.await.unwrap();
    assert_eq!(requests[0]["tools"].as_array().unwrap().len(), 1);
    assert!(requests[0].get("thinking").is_none());
    for request in &requests {
        assert!(!request.to_string().contains("PRIVATE_UNSELECTED_ROW"));
    }
    let tools = requests[2]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "tool")
        .collect::<Vec<_>>();
    let result: Value = serde_json::from_str(tools[1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(result["result"], json!({"before":0,"after":"Count: 1"}));
    assert!(result.get("state").is_none());
}

#[tokio::test]
async fn deepseek_thinking_payload_is_retained_across_tool_rounds() {
    let directory = tempfile::tempdir().unwrap();
    let mut executor = executor(directory.path(), false);
    let reasoning = "opaque-provider-payload\n原样保留";
    let mut first = tool("first", "state.n=41; return state.n;");
    first["choices"][0]["message"]["reasoning_content"] = json!(reasoning);
    let mut second = tool("second", "return state.n+1;");
    second["choices"][0]["message"]["reasoning_content"] = json!("");
    let (base, server) = scripted_model(vec![first, second, done("42")]).await;
    let model = Model::new(&base, "deepseek-flash".into(), None)
        .unwrap()
        .with_thinking(true);
    let outcome = agent::run(&model, &mut executor, "Compute 42", Limits::default())
        .await
        .unwrap();
    assert_eq!(outcome.answer, "42");
    let requests = server.await.unwrap();
    for request in &requests {
        assert_eq!(request["thinking"], json!({"type":"enabled"}));
    }
    assert_eq!(requests[1]["messages"][2]["reasoning_content"], reasoning);
    assert!(requests[1]["messages"][2]["content"].is_null());
    assert_eq!(requests[2]["messages"][2]["reasoning_content"], reasoning);
    assert_eq!(requests[2]["messages"][4]["reasoning_content"], "");
}

#[tokio::test]
async fn deepseek_non_thinking_mode_can_be_selected_explicitly() {
    let directory = tempfile::tempdir().unwrap();
    let mut executor = executor(directory.path(), false);
    let (base, server) = scripted_model(vec![done("Ready")]).await;
    let model = Model::new(&base, "deepseek-flash".into(), None)
        .unwrap()
        .with_thinking(false);
    agent::run(&model, &mut executor, "Say ready", Limits::default())
        .await
        .unwrap();
    assert_eq!(
        server.await.unwrap()[0]["thinking"],
        json!({"type":"disabled"})
    );
}

#[tokio::test]
async fn permissions_are_enforced_outside_javascript() {
    let directory = tempfile::tempdir().unwrap();
    let host = host(directory.path()).await;
    let mut executor = executor(directory.path(), false);
    let denied = executor
        .execute("await android.ui.act({node:'increment',action:'click'});")
        .await;
    assert!(!denied.ok && denied.error.unwrap().contains("Actions are disabled"));
    assert_eq!(host.calls.load(Ordering::Relaxed), 0);
    executor.config.allow_actions = true;
    let denied = executor
        .execute("await android.notification.reply({key:'n',text:'no'});")
        .await;
    assert!(!denied.ok && denied.error.unwrap().contains("replies are disabled"));
    let bypass = executor
        .execute("return JSON.parse(__rpc('host.read_secret','{}'));")
        .await;
    assert!(
        bypass.result["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not exposed")
    );
    assert_eq!(host.calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn screenshots_are_private_artifacts_not_base64_in_model_results() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let _host = host(directory.path()).await;
    let mut executor = executor(directory.path(), false);
    let result = executor
        .execute("return await android.ui.screenshot({max_width:320});")
        .await;
    assert!(result.ok, "{:?}", result.error);
    assert_eq!(result.result["artifact"], "screenshot-1.png");
    let path = directory.path().join("artifacts/screenshot-1.png");
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(!serde_json::to_string(&result).unwrap().contains("iVBOR"));
}

#[tokio::test]
async fn deadline_kills_worker_even_when_rpc_is_blocked_and_state_is_not_committed() {
    let directory = tempfile::tempdir().unwrap();
    let host = host(directory.path()).await;
    let mut executor = executor(directory.path(), true);
    assert!(executor.execute("state.kept='yes'; return null;").await.ok);
    executor.config.timeout_ms = 200;
    let started = Instant::now();
    let result = executor.execute("state.kept='no'; await android.ui.act({node:'increment',action:'click'}); await android.ui.wait({until:'text',value:'blocked'});").await;
    assert!(!result.ok && result.error.unwrap().contains("NOT rolled back"));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(host.count.load(Ordering::Relaxed), 1);
    assert_eq!(executor.execute("return state.kept;").await.result, "yes");
}

#[tokio::test]
async fn malformed_arguments_can_be_repaired_without_executing_unknown_tools() {
    let directory = tempfile::tempdir().unwrap();
    let mut executor = executor(directory.path(), false);
    let mut bad = tool("bad", "return 1;");
    bad["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"] = json!("not JSON");
    let (base, server) = scripted_model(vec![bad, tool("retry", "return 42;"), done("42")]).await;
    let outcome = agent::run(
        &Model::new(&base, "fixture".into(), None).unwrap(),
        &mut executor,
        "Compute 42",
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(outcome.code_executions, 1);
    assert_eq!(outcome.rpc_calls, 0);
    assert!(
        server.await.unwrap()[1]
            .to_string()
            .contains("no code executed")
    );
    let mut bad = tool("unknown", "return 1;");
    bad["choices"][0]["message"]["tool_calls"][0]["function"]["name"] = json!("shell");
    let (base, server) = scripted_model(vec![bad]).await;
    assert!(
        agent::run(
            &Model::new(&base, "fixture".into(), None).unwrap(),
            &mut executor,
            "Test",
            Limits::default()
        )
        .await
        .is_err()
    );
    server.await.unwrap();
}

#[tokio::test]
async fn explicitly_requested_step_limit_does_not_report_unfinished_tasks_as_success() {
    let directory = tempfile::tempdir().unwrap();
    let mut executor = executor(directory.path(), false);
    let (base, server) = scripted_model(vec![tool("one", "return 1;")]).await;
    let model = Model::new(&base, "fixture".into(), None).unwrap();
    assert!(
        agent::run(&model, &mut executor, "Test", Limits { steps: Some(1) })
            .await
            .unwrap_err()
            .to_string()
            .contains("--max-steps limit")
    );
    server.await.unwrap();
}

#[tokio::test]
async fn default_run_continues_without_step_rpc_or_context_budgets() {
    let directory = tempfile::tempdir().unwrap();
    let host = host(directory.path()).await;
    let mut executor = executor(directory.path(), false);
    let mut replies = Vec::new();
    for step in 0..13 {
        let calls = if step == 0 { 40 } else { 11 };
        let code = format!(
            "for(let i=0;i<{calls};i++) await android.app.list({{}}); state.n=(state.n||0)+1; return state.n; /*{}*/",
            "x".repeat(16 * 1024)
        );
        replies.push(tool(&format!("step-{step}"), &code));
    }
    replies.push(done("Finished after thirteen code executions"));
    let (base, server) = scripted_model(replies).await;
    let outcome = agent::run(
        &Model::new(&base, "fixture".into(), None).unwrap(),
        &mut executor,
        "Continue until done",
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(outcome.turns, 1);
    assert_eq!(outcome.steps, 14);
    assert_eq!(outcome.code_executions, 13);
    assert_eq!(outcome.rpc_calls, 172);
    assert_eq!(host.calls.load(Ordering::Relaxed), 172);
    assert_eq!(outcome.answer, "Finished after thirteen code executions");
    let requests = server.await.unwrap();
    assert!(requests.last().unwrap()["messages"].to_string().len() > 192 * 1024);
}

#[test]
fn model_urls_do_not_allow_remote_cleartext_or_url_credentials() {
    for base in [
        "http://example.com/v1",
        "https://token@example.com/v1",
        "https://example.com/v1?key=secret",
    ] {
        assert!(Model::new(base, "test".into(), None).is_err());
    }
}

#[test]
fn cli_api_needs_no_model_device_or_running_host() {
    let directory = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_gui-use-agent"))
        .args(["--state-dir", directory.path().to_str().unwrap(), "api"])
        .env("GUI_USE_ADB", "/missing/adb")
        .output()
        .unwrap();
    assert!(output.status.success());
    let catalog: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        catalog.as_array().unwrap().len(),
        gui_use_protocol::METHODS.len()
    );
    assert!(!directory.path().join("pairing.json").exists());
}

#[test]
fn missing_deepseek_key_fails_before_initializing_host_or_touching_usb() {
    let directory = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_gui-use-agent"))
        .args([
            "--state-dir",
            directory.path().to_str().unwrap(),
            "run",
            "--base-url",
            "https://api.deepseek.com",
            "list apps",
        ])
        .env_remove("GUI_USE_API_KEY_FILE")
        .env_remove("GUI_USE_API_KEY")
        .env_remove("DEEPSEEK_API_KEY")
        .env("GUI_USE_ADB", "/missing/adb")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("请先设置 DEEPSEEK_API_KEY"));
    assert!(!directory.path().join("pairing.json").exists());
}

#[tokio::test]
async fn tracing_reports_each_llm_usage_and_rpc_in_steps_of_one_turn() {
    let directory = tempfile::tempdir().unwrap();
    let _host = host(directory.path()).await;
    let mut executor = executor(directory.path(), false);
    let mut first = tool(
        "inspect",
        "const s=await android.ui.snapshot({filter:'all'}); return {nodes:s.nodes.length};",
    );
    first["usage"] = json!({"prompt_tokens":100,"completion_tokens":20,"total_tokens":120,
        "prompt_cache_hit_tokens":80,"prompt_cache_miss_tokens":20,
        "completion_tokens_details":{"reasoning_tokens":6},"cost":0.01});
    let mut last = done("1002 nodes");
    last["usage"] = json!({"prompt_tokens":50,"completion_tokens":4,"total_tokens":54,
        "prompt_cache_hit_tokens":40,"prompt_cache_miss_tokens":10,
        "completion_tokens_details":{"reasoning_tokens":0},"cost":0.005});
    let (base, server) = scripted_model(vec![first, last]).await;
    let model = Model::new(
        &base,
        "fixture".into(),
        Some("MODEL_KEY_MUST_NOT_APPEAR".into()),
    )
    .unwrap();
    let logs = LogBuffer::default();
    let outcome = agent::run(&model, &mut executor, "Inspect nodes", Limits::default())
        .with_subscriber(logs.subscriber())
        .await
        .unwrap();
    assert_eq!(outcome.turns, 1);
    assert_eq!(outcome.steps, 2);
    assert_eq!(outcome.usage["total_tokens"], 174);
    assert_eq!(outcome.usage["prompt_cache_hit_tokens"], 120);
    assert_eq!(
        outcome.usage["completion_tokens_details"]["reasoning_tokens"],
        6
    );
    assert!((outcome.usage["cost"].as_f64().unwrap() - 0.015).abs() < 0.000001);
    let events = logs.events();
    let llm = events
        .iter()
        .filter(|e| e["fields"]["event"] == "llm.completed")
        .collect::<Vec<_>>();
    assert_eq!(llm.len(), 2); // Includes the final answer, not just tool-calling steps.
    assert_eq!(llm[0]["fields"]["input_tokens"], 100);
    assert_eq!(llm[0]["fields"]["cached_tokens"], 80);
    assert_eq!(llm[0]["fields"]["reasoning_tokens"], 6);
    assert_eq!(llm[1]["fields"]["output_tokens"], 4);
    for (index, event) in llm.iter().enumerate() {
        let spans = event["spans"].as_array().unwrap();
        assert!(
            spans
                .iter()
                .any(|span| span["name"] == "agent_turn" && span["turn"] == 1)
        );
        assert!(
            spans
                .iter()
                .any(|span| span["name"] == "step" && span["step"] == index + 1)
        );
    }
    let request = events
        .iter()
        .find(|e| e["fields"]["event"] == "rpc.request")
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(request["fields"]["params"].as_str().unwrap()).unwrap(),
        json!({"filter":"all"})
    );
    assert_eq!(request["span"]["method"], "ui.snapshot");
    let response = events
        .iter()
        .find(|e| e["fields"]["event"] == "rpc.response")
        .unwrap();
    let result: Value =
        serde_json::from_str(response["fields"]["result"].as_str().unwrap()).unwrap();
    assert_eq!(result["node_count"], 1002);
    let text = events.iter().map(Value::to_string).collect::<String>();
    assert!(!text.contains("MODEL_KEY_MUST_NOT_APPEAR"));
    assert!(!text.contains("PRIVATE_UNSELECTED_ROW")); // Full trees require DEBUG.
    server.await.unwrap();
}

#[tokio::test]
async fn tracing_marks_missing_usage_and_failed_requests_without_inventing_cost() {
    let (base, server) = scripted_model(vec![
        done("ok"),
        json!({"error":"RAW_BODY_MUST_NOT_BE_LOGGED"}),
    ])
    .await;
    let model = Model::new(&base, "fixture".into(), None).unwrap();
    let logs = LogBuffer::default();
    model
        .complete(&[json!({"role":"user","content":"test"})])
        .with_subscriber(logs.subscriber())
        .await
        .unwrap();
    assert!(
        model
            .complete(&[json!({"role":"user","content":"test"})])
            .with_subscriber(logs.subscriber())
            .await
            .is_err()
    );
    let events = logs.events();
    let success = events
        .iter()
        .find(|e| e["fields"]["event"] == "llm.completed")
        .unwrap();
    assert_eq!(success["fields"]["usage_reported"], false);
    assert!(success["fields"].get("input_tokens").is_none());
    assert!(events.iter().any(|e| e["fields"]["event"] == "llm.failed"));
    assert!(
        !events
            .iter()
            .map(Value::to_string)
            .collect::<String>()
            .contains("RAW_BODY_MUST_NOT_BE_LOGGED")
    );
    server.await.unwrap();
}

#[tokio::test]
async fn tracing_records_denied_and_interrupted_rpc_calls() {
    let directory = tempfile::tempdir().unwrap();
    let _host = host(directory.path()).await;
    let mut executor = executor(directory.path(), false);
    let logs = LogBuffer::default();
    assert!(
        !executor
            .execute("await android.ui.act({node:'increment',action:'click'});")
            .with_subscriber(logs.subscriber())
            .await
            .ok
    );
    executor.config.timeout_ms = 100;
    assert!(
        !executor
            .execute("await android.ui.wait({until:'text',value:'blocked'});")
            .with_subscriber(logs.subscriber())
            .await
            .ok
    );
    let events = logs.events();
    assert_eq!(
        events
            .iter()
            .filter(|e| e["fields"]["event"] == "rpc.request")
            .count(),
        2
    );
    assert!(events.iter().any(|e| e["fields"]["event"] == "rpc.error"));
    assert!(
        events
            .iter()
            .any(|e| e["fields"]["event"] == "rpc.cancelled")
    );
}
