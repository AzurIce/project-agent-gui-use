use gui_use_agent::worker::{Input, OUTPUT_LIMIT, evaluate};
use serde_json::{Value, json};

fn run(code: &str) -> gui_use_agent::worker::Output {
    evaluate(
        Input {
            code: code.into(),
            state: json!({}),
            timeout_ms: 500,
        },
        |_, _| panic!("No RPC expected"),
    )
    .unwrap()
}

#[test]
fn only_selected_values_leave_the_runtime_and_state_is_separate() {
    let output = evaluate(Input { code: "const s = await android.ui.snapshot({filter:'all'}); state.total=s.nodes.length; return s.nodes.filter(n=>n.text==='wanted').map(n=>({text:n.text}));".into(), state: json!({}), timeout_ms: 500 }, |method, params| {
        assert_eq!(method, "ui.snapshot");
        assert_eq!(params, json!({"filter":"all"}));
        Ok(json!({"result":{"nodes":[{"text":"private-unselected-data"},{"text":"wanted"}]}}))
    }).unwrap();
    assert!(output.error.is_none(), "{:?}", output.error);
    assert_eq!(output.result, json!([{"text":"wanted"}]));
    assert_eq!(output.state, json!({"total":2}));
    assert!(
        !output
            .result
            .to_string()
            .contains("private-unselected-data")
    );
}

#[test]
fn javascript_has_no_files_process_network_or_import_capabilities() {
    let output = run(
        "return [typeof process, typeof require, typeof fetch, typeof Deno, typeof std, typeof os, typeof setTimeout];",
    );
    assert_eq!(output.result, json!(vec!["undefined"; 7]));
    let output = run("await import('node:fs');");
    assert!(output.error.is_some());
}

#[test]
fn output_is_capped_for_return_and_print_without_broken_unicode() {
    let output = run("print('界'.repeat(10000)); return 'too much';");
    assert!(output.truncated);
    assert!(output.output.iter().map(String::len).sum::<usize>() <= OUTPUT_LIMIT);
    assert!(output.error.is_some());
    let output = run("return 'x'.repeat(20000);");
    assert_eq!(output.result, Value::Null);
    assert!(output.truncated && output.error.is_some());
}

#[test]
fn infinite_loops_and_excessive_allocations_are_stopped() {
    let started = std::time::Instant::now();
    let output = evaluate(
        Input {
            code: "while(true) {}".into(),
            state: json!({}),
            timeout_ms: 20,
        },
        |_, _| panic!(),
    )
    .unwrap();
    assert!(output.error.is_some());
    assert!(started.elapsed().as_secs() < 2);
    let output = run(
        "const a=[]; for(let i=0;i<10000;i++) a.push(new Array(10000).fill(i)); return a.length;",
    );
    assert!(output.error.is_some());
}

#[test]
fn errors_and_rejected_promises_are_reported_with_no_implicit_return_of_data() {
    assert!(run("const = bad syntax").error.is_some());
    assert!(
        run("throw new Error('expected failure');")
            .error
            .unwrap()
            .contains("expected failure")
    );
    assert!(run("await new Promise(()=>{});").error.is_some());
    assert!(run("return 4n;").error.is_some());
    let output = run("state.a=1; throw new Error('failed');");
    assert_eq!(output.state, json!({}));
}

#[test]
fn rpc_failures_remain_catchable_errors_with_codes() {
    let output = evaluate(Input { code: "try { await android.ui.act({node:'stale',action:'click'}); } catch(e) { return {code:e.code,message:e.message}; }".into(), state: json!({}), timeout_ms: 500 }, |_, _| {
        Ok(json!({"error":{"code":2001,"message":"Node is stale"}}))
    }).unwrap();
    assert_eq!(
        output.result,
        json!({"code":2001,"message":"Node is stale"})
    );
}

#[test]
fn help_and_sdk_are_derived_from_the_rust_registry() {
    let output = run(
        "return {count:android.help().length, required:android.help('ui.act').params.required};",
    );
    assert_eq!(output.result["count"], gui_use_protocol::METHODS.len());
    assert!(
        output.result["required"]
            .as_array()
            .unwrap()
            .contains(&json!("node"))
    );
}
