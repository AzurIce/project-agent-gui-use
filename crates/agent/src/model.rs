use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use reqwest::{Client, Url};
use serde_json::{Value, json};

pub struct Model {
    client: Client,
    endpoint: Url,
    name: String,
    thinking: Option<bool>,
    // Kept in the parent, never serialized into a prompt or passed to the worker.
    key: Option<String>,
}

impl Model {
    pub fn new(base_url: &str, name: String, key: Option<String>) -> Result<Self> {
        ensure!(!name.is_empty(), "A model name is required");
        let mut base = Url::parse(base_url).context("Invalid model base URL")?;
        ensure!(
            base.username().is_empty()
                && base.password().is_none()
                && base.query().is_none()
                && base.fragment().is_none(),
            "Model URL must not contain credentials, query parameters or fragments"
        );
        let local = matches!(
            base.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
        );
        ensure!(
            base.scheme() == "https" || (local && base.scheme() == "http"),
            "Model API requires HTTPS, except loopback HTTP"
        );
        base.set_path(&format!(
            "{}/chat/completions",
            base.path().trim_end_matches('/')
        ));
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(15))
            .build()?;
        Ok(Self {
            client,
            endpoint: base,
            name,
            thinking: None,
            key,
        })
    }

    /// Omit by default for generic OpenAI-compatible providers. DeepSeek supports
    /// an explicit switch and requires reasoning_content to be retained in history.
    pub fn with_thinking(mut self, enabled: bool) -> Self {
        self.thinking = Some(enabled);
        self
    }

    pub async fn complete(&self, messages: &[Value]) -> Result<Value> {
        let started = Instant::now();
        let response = self.request(messages).await;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match &response {
            Ok(response) => {
                let usage = &response["usage"];
                let cached_tokens = usage["prompt_cache_hit_tokens"]
                    .as_u64()
                    .or_else(|| usage["prompt_tokens_details"]["cached_tokens"].as_u64());
                tracing::info!(event = "llm.completed", model = %self.name, elapsed_ms,
                    response_id = response["id"].as_str(),
                    finish_reason = response["choices"][0]["finish_reason"].as_str(),
                    usage_reported = usage.is_object(),
                    input_tokens = usage["prompt_tokens"].as_u64(),
                    output_tokens = usage["completion_tokens"].as_u64(),
                    total_tokens = usage["total_tokens"].as_u64(),
                    cached_tokens,
                    cache_miss_tokens = usage["prompt_cache_miss_tokens"].as_u64(),
                    reasoning_tokens = usage["completion_tokens_details"]["reasoning_tokens"].as_u64(),
                    usage = %usage, "LLM request completed");
            }
            Err(error) => tracing::warn!(event = "llm.failed", model = %self.name, elapsed_ms,
                error = %error, "LLM request failed; usage unavailable"),
        }
        response
    }

    async fn request(&self, messages: &[Value]) -> Result<Value> {
        let mut body = json!({"model":self.name,"messages":messages,"stream":false,
            "tools":[{"type":"function","function":{"name":"android_code",
                "description":"Execute JavaScript to compose Android calls and return only selected evidence. No shell, files or arbitrary network. Only print and return reach model context.",
                "parameters":{"type":"object","properties":{"code":{"type":"string"}},"required":["code"],"additionalProperties":false}}}],
            "tool_choice":"auto","parallel_tool_calls":false});
        if let Some(enabled) = self.thinking {
            body["thinking"] = json!({"type":if enabled { "enabled" } else { "disabled" }});
        }
        let mut request = self.client.post(self.endpoint.clone()).json(&body);
        if let Some(key) = &self.key {
            request = request.bearer_auth(key);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Model request failed (transport details redacted)"))?;
        ensure!(
            response.status().is_success(),
            "Model API returned HTTP {} (body redacted)",
            response.status().as_u16()
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow::anyhow!("Cannot read model response"))?
        {
            ensure!(
                bytes.len() + chunk.len() <= 512 * 1024,
                "Model response exceeds 512 KiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).context("Model returned invalid JSON")?;
        ensure!(
            value["choices"][0]["message"].is_object(),
            "Model response has no assistant message"
        );
        Ok(value)
    }
}
