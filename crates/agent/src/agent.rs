use anyhow::{Result, ensure};
use serde::Serialize;
use serde_json::{Value, json};
use tracing::Instrument;

use crate::{api, executor::Executor, model::Model, worker::CODE_LIMIT};

#[derive(Default)]
pub struct Limits {
    /// No step limit unless the caller explicitly opts in.
    pub steps: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct Outcome {
    pub answer: String,
    pub turns: usize,
    pub steps: usize,
    pub code_executions: usize,
    pub rpc_calls: usize,
    pub rpc_response_bytes: usize,
    pub tool_result_bytes: usize,
    pub usage: Value,
}

#[tracing::instrument(name = "agent_turn", skip_all, fields(turn = 1))]
pub async fn run(
    model: &Model,
    executor: &mut Executor,
    task: &str,
    limits: Limits,
) -> Result<Outcome> {
    ensure!(
        !task.trim().is_empty() && task.len() <= 16 * 1024,
        "Task must contain 1..16384 bytes"
    );
    let mut messages = vec![
        json!({"role":"system","content":api::system_prompt(executor.config.allow_actions, executor.config.allow_notification_reply)}),
        json!({"role":"user","content":task}),
    ];
    let mut outcome = Outcome {
        answer: String::new(),
        turns: 1,
        steps: 0,
        code_executions: 0,
        rpc_calls: 0,
        rpc_response_bytes: 0,
        tool_result_bytes: 0,
        usage: Value::Null,
    };
    loop {
        if let Some(max) = limits.steps {
            ensure!(
                outcome.steps < max,
                "Explicit --max-steps limit reached ({max}); task is not reported as complete"
            );
        }
        let step = outcome.steps + 1;
        let step_span = tracing::info_span!("step", step);
        let response = model
            .complete(&messages)
            .instrument(step_span.clone())
            .await?;
        outcome.steps = step;
        add_usage(&mut outcome.usage, &response["usage"]);
        let message = &response["choices"][0]["message"];
        ensure!(message["role"] == "assistant", "Expected assistant role");
        let calls = message["tool_calls"]
            .as_array()
            .filter(|calls| !calls.is_empty());
        let Some(calls) = calls else {
            ensure!(
                response["choices"][0]["finish_reason"] == "stop",
                "Model stopped without a complete answer"
            );
            let answer = message["content"].as_str().unwrap_or("");
            ensure!(
                !answer.is_empty() && answer.len() <= 16 * 1024,
                "Missing or oversized final answer"
            );
            outcome.answer = answer.into();
            tracing::info!(parent: &step_span, event = "step.completed", tool_calls = 0, "Step completed; turn finished");
            return Ok(outcome);
        };
        ensure!(
            response["choices"][0]["finish_reason"] == "tool_calls",
            "Incomplete tool call; nothing was executed"
        );
        // Validate the entire batch BEFORE executing anything. Parallel UI actions are unsafe.
        ensure!(
            calls.len() == 1,
            "Only one android_code call per step is supported; nothing was executed"
        );
        let call = &calls[0];
        ensure!(
            call["type"] == "function" && call["function"]["name"] == "android_code",
            "Unknown tool; nothing was executed"
        );
        let id = call["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .ok_or_else(|| anyhow::anyhow!("Invalid tool call ID"))?;
        let arguments = call["function"]["arguments"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Tool arguments must be a JSON string"))?;
        ensure!(
            arguments.len() <= CODE_LIMIT * 2,
            "Tool arguments exceed limit"
        );
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Arguments {
            code: String,
        }
        let arguments: Result<Arguments, _> = serde_json::from_str(arguments);
        let mut history =
            json!({"role":"assistant","content":message["content"],"tool_calls":calls});
        // DeepSeek thinking-mode tool calls require the exact reasoning payload
        // on subsequent requests. Keep it in history, not logs or tool results.
        if let Some(reasoning) = message.get("reasoning_content") {
            history["reasoning_content"] = reasoning.clone();
        }
        messages.push(history);
        let result = match arguments {
            Ok(arguments) => {
                let result = executor.execute(&arguments.code).instrument(step_span.clone()).await;
                outcome.code_executions += 1;
                outcome.rpc_calls += result.stats.rpc_calls;
                outcome.rpc_response_bytes += result.stats.rpc_response_bytes;
                serde_json::to_string(&result)?
            }
            Err(_) => json!({"ok":false,"error":"Expected exactly {\"code\":\"JavaScript body\"}; no code executed"}).to_string(),
        };
        outcome.tool_result_bytes += result.len();
        tracing::info!(parent: &step_span, event = "step.completed", tool_calls = 1,
            rpc_calls_total = outcome.rpc_calls, raw_bytes_total = outcome.rpc_response_bytes,
            result_bytes_total = outcome.tool_result_bytes, "Step completed");
        messages.push(json!({"role":"tool","tool_call_id":id,"content":result}));
    }
}

/// Sum reported usage, including nested cache/reasoning counters and provider costs.
/// Missing usage stays unknown rather than being reported as zero.
fn add_usage(total: &mut Value, usage: &Value) {
    let Some(fields) = usage.as_object() else {
        return;
    };
    if !total.is_object() {
        *total = json!({});
    }
    for (name, value) in fields {
        if value.is_object() {
            add_usage(&mut total[name], value);
        } else if let Some(number) = value.as_u64() {
            total[name] = json!(total[name].as_u64().unwrap_or(0).saturating_add(number));
        } else if let Some(number) = value.as_f64() {
            total[name] = json!(total[name].as_f64().unwrap_or(0.0) + number);
        }
    }
}
