//! Disposable QuickJS worker. Its only I/O capability is parent-brokered Android RPC.
//! The parent clears its environment and kills it on deadline; no model credential is sent.

use std::cell::RefCell;
use std::io::{self, BufRead, Read, Write};
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{Result, ensure};
use rquickjs::{Context, Ctx, Function, Promise, Runtime};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{api, prefix};

pub const CODE_LIMIT: usize = 32 * 1024;
pub const OUTPUT_LIMIT: usize = 8 * 1024;
pub const STATE_LIMIT: usize = 64 * 1024;
pub const FRAME_LIMIT: usize = 128 * 1024;

#[derive(Serialize, Deserialize)]
pub struct Input {
    pub code: String,
    pub state: Value,
    pub timeout_ms: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Output {
    pub result: Value,
    pub output: Vec<String>,
    pub truncated: bool,
    pub error: Option<String>,
    pub state: Value,
}

pub fn send(value: &Value) -> Result<()> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, value)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

/// The stdin reader lives in the worker; no file/module loader is registered with QuickJS.
pub fn run() -> Result<()> {
    let mut line = String::new();
    io::stdin()
        .lock()
        .take((FRAME_LIMIT + 1) as u64)
        .read_line(&mut line)?;
    ensure!(line.len() <= FRAME_LIMIT, "Worker input exceeds limit");
    let input: Input = serde_json::from_str(&line)?;
    let output = evaluate(input, |method, params| {
        send(&json!({"kind":"rpc","method":method,"params":params}))?;
        let mut line = String::new();
        // Broker may return a large tree, but never screenshots as base64.
        io::stdin()
            .lock()
            .take(16 * 1024 * 1024 + 1)
            .read_line(&mut line)?;
        ensure!(
            line.len() <= 16 * 1024 * 1024,
            "Broker response exceeds limit"
        );
        Ok(serde_json::from_str(&line)?)
    })?;
    send(&json!({"kind":"done","value":output}))
}

pub fn evaluate<F>(input: Input, rpc: F) -> Result<Output>
where
    F: FnMut(String, Value) -> Result<Value> + 'static,
{
    ensure!(input.code.len() <= CODE_LIMIT, "Code exceeds 32 KiB");
    let runtime =
        Runtime::new().map_err(|_| anyhow::anyhow!("Cannot create JavaScript runtime"))?;
    runtime.set_memory_limit(64 * 1024 * 1024);
    runtime.set_max_stack_size(512 * 1024);
    let deadline = Instant::now() + Duration::from_millis(input.timeout_ms);
    runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
    let context =
        Context::full(&runtime).map_err(|_| anyhow::anyhow!("Cannot create JavaScript context"))?;
    let lines = Rc::new(RefCell::new(Vec::<String>::new()));
    let remaining = Rc::new(RefCell::new(OUTPUT_LIMIT));
    let truncated = Rc::new(RefCell::new(false));
    let mut next_state = input.state.clone();
    let execution = context.with(|ctx| -> rquickjs::Result<String> {
        let callback = RefCell::new(rpc);
        ctx.globals().set("__rpc", Function::new(ctx.clone(), move |method: String, params: String| -> String {
            let response = (|| -> Result<Value> {
                ensure!(params.len() <= CODE_LIMIT, "RPC parameters exceed limit");
                (callback.borrow_mut())(method, serde_json::from_str(&params)?)
            })();
            match response {
                Ok(value) => value.to_string(),
                Err(_) => json!({"error":{"code":-32000,"message":"Broker I/O failed"}}).to_string(),
            }
        })?)?;
        let printed = lines.clone();
        let budget = remaining.clone();
        let cut = truncated.clone();
        ctx.globals().set("__print", Function::new(ctx.clone(), move |line: String| {
            let mut left = budget.borrow_mut();
            if line.len() > *left || printed.borrow().len() >= 64 { *cut.borrow_mut() = true; }
            if *left > 0 && printed.borrow().len() < 64 {
                let mut line = prefix(&line, *left).to_owned();
                // Budget JSON-encoded bytes, including escaped quotes/control chars.
                while !line.is_empty() && serde_json::to_string(&line).map_or(usize::MAX, |s| s.len() + 1) > *left {
                    line.pop();
                    *cut.borrow_mut() = true;
                }
                let cost = serde_json::to_string(&line).map_or(usize::MAX, |s| s.len() + 1);
                if cost > *left { return; }
                *left -= cost;
                printed.borrow_mut().push(line);
            }
        })?)?;
        ctx.globals().set("__catalog", api::catalog().to_string())?;
        ctx.globals().set("__state", input.state.to_string())?;
        ctx.globals().set("__help", Function::new(ctx.clone(), |name: String| api::help(&name).to_string())?)?;
        ctx.eval::<(), _>(include_str!("runtime.js"))?;
        let source = format!("(async () => {{ const result = await (async () => {{\n{}\n}})(); return JSON.stringify({{result: result === undefined ? null : result, state}}); }})()", input.code);
        let value = ctx.eval::<Promise, _>(source).and_then(|promise| promise.finish::<String>());
        match value {
            Ok(value) => Ok(value),
            Err(error) => {
                // Extract within the engine deadline, not by unboundedly formatting a user object.
                let description = exception_message(&ctx, &error);
                Err(rquickjs::Error::new_from_js_message("JavaScript", "execution", description))
            }
        }
    });
    let (result, error) = match execution {
        Ok(serialized) => {
            let mut value: Value = serde_json::from_str(&serialized)?;
            let state = value["state"].take();
            let result = value["result"].take();
            if state.to_string().len() > STATE_LIMIT {
                (
                    Value::Null,
                    Some("state exceeds 64 KiB; previous state retained".into()),
                )
            } else {
                if result.to_string().len() > *remaining.borrow() {
                    *truncated.borrow_mut() = true;
                    (
                        Value::Null,
                        Some("Returned value exceeds output budget; select fewer fields".into()),
                    )
                } else {
                    next_state = state;
                    (result, None)
                }
            }
        }
        Err(error) => (
            Value::Null,
            Some(prefix(&error.to_string(), 1024).to_owned()),
        ),
    };
    let output = lines.borrow().clone();
    let truncated = *truncated.borrow();
    Ok(Output {
        result,
        output,
        truncated,
        error,
        state: next_state,
    })
}

fn exception_message(ctx: &Ctx<'_>, error: &rquickjs::Error) -> String {
    if error.is_exception() {
        let caught = ctx.catch();
        if let Some(exception) = caught.as_exception() {
            return prefix(
                &exception
                    .message()
                    .unwrap_or_else(|| "JavaScript exception".into()),
                1024,
            )
            .to_owned();
        }
        return "JavaScript threw a non-Error value".into();
    }
    error.to_string()
}
