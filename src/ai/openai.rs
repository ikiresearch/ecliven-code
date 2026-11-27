use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::ai::{
    AiError, CompletionRequest, CompletionResponse, LlmProvider, Message, PromptCache, Role,
    TokenUsage, ToolCall,
};

pub struct OpenAiCompatibleProvider {
    client: Client,
    provider_name: &'static str,
    api_key: String,
    base_url: String,
    model: String,
    is_openrouter: bool,
    cache: PromptCache,
}

impl OpenAiCompatibleProvider {
    pub fn new(
        provider_name: &'static str,
        api_key: String,
        base_url: String,
        model: String,
        is_openrouter: bool,
    ) -> Self {
        Self {
            client: Client::new(),
            provider_name,
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
            is_openrouter,
            cache: PromptCache::default(),
        }
    }

    pub fn with_prompt_cache(mut self, cache: PromptCache) -> Self {
        self.cache = cache;
        self
    }

    /// Models behind OpenRouter that only cache with explicit breakpoints.
    fn needs_breakpoints(&self) -> bool {
        self.is_openrouter
            && (self.model.starts_with("anthropic/") || self.model.starts_with("google/gemini"))
    }

    fn build_payload(&self, request: &CompletionRequest, stream: bool) -> Value {
        let mut messages: Vec<Value> = Vec::new();
        if !request.system_prompt.is_empty() {
            messages.push(json!({
                "role": "system",
                "content": request.system_prompt,
            }));
        }

        for msg in &request.messages {
            messages.push(convert_message(msg));
        }

        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect();

        if self.cache != PromptCache::Off && self.needs_breakpoints() {
            let cc = json!({ "type": "ephemeral" });
            for idx in [0, messages.len().saturating_sub(1)] {
                let Some(msg) = messages.get_mut(idx) else { continue };
                let role = msg.get("role").and_then(Value::as_str).unwrap_or("");
                if !matches!(role, "system" | "user") {
                    continue;
                }
                if let Some(Value::String(text)) = msg.get_mut("content") {
                    if !text.is_empty() {
                        let text = std::mem::take(text);
                        msg["content"] = json!([{ "type": "text", "text": text, "cache_control": cc }]);
                    }
                }
            }
        }

        let mut payload = json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": request.max_tokens,
            "temperature": request.temperature,
            "stream": stream,
        });
        if stream {
            payload["stream_options"] = json!({ "include_usage": true });
        }
        if self.cache != PromptCache::Off && !self.is_openrouter {
            // Routes requests with the same prefix to the same cache.
            payload["prompt_cache_key"] = json!(prompt_cache_key(&request.system_prompt, &self.model));
        }

        if !tools.is_empty() {
            payload["tools"] = Value::Array(tools);
        }

        payload
    }
}

fn prompt_cache_key(system_prompt: &str, model: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{model}\n{system_prompt}").as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("ecliven-{hex}")
}

fn usage_from(u: &Value) -> Option<TokenUsage> {
    let get = |p: &str| u.pointer(p).and_then(Value::as_u64).map(|v| v as u32);
    Some(TokenUsage {
        input_tokens: get("/prompt_tokens")?,
        output_tokens: get("/completion_tokens")?,
        cache_read_tokens: get("/prompt_tokens_details/cached_tokens").unwrap_or(0),
        cache_write_tokens: get("/prompt_tokens_details/cache_write_tokens").unwrap_or(0),
    })
}

fn convert_message(msg: &Message) -> Value {
    match msg.role {
        Role::System => json!({
            "role": "system",
            "content": msg.content,
        }),
        Role::User if !msg.images.is_empty() => {
            let mut parts = vec![json!({ "type": "text", "text": msg.content })];
            parts.extend(msg.images.iter().map(|img| {
                json!({ "type": "image_url", "image_url": { "url": img.data_url() } })
            }));
            json!({ "role": "user", "content": parts })
        }
        Role::User => json!({
            "role": "user",
            "content": msg.content,
        }),
        Role::Assistant => {
            if msg.tool_calls.is_empty() {
                json!({
                    "role": "assistant",
                    "content": msg.content,
                })
            } else {
                let calls: Vec<Value> = msg
                    .tool_calls
                    .iter()
                    .map(|tc| {
                        json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": tc.arguments.to_string(),
                            }
                        })
                    })
                    .collect();
                json!({
                    "role": "assistant",
                    "content": if msg.content.is_empty() { Value::Null } else { Value::String(msg.content.clone()) },
                    "tool_calls": calls,
                })
            }
        }
        Role::Tool => json!({
            "role": "tool",
            "tool_call_id": msg.tool_call_id.as_deref().unwrap_or("unknown"),
            "content": msg.content,
        }),
    }
}

#[derive(Default)]
struct PartialOpenAiToolCall {
    id: String,
    name: String,
    arguments_json: String,
}

#[async_trait]
impl LlmProvider for OpenAiCompatibleProvider {
    fn name(&self) -> &'static str {
        self.provider_name
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn complete(
        &self,
        request: &CompletionRequest,
        delta_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<CompletionResponse, AiError> {
        let streaming = delta_tx.is_some();
        let url = format!("{}/chat/completions", self.base_url);
        let payload = self.build_payload(request, streaming);

        let mut req_builder = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .header("content-type", "application/json")
            .json(&payload);

        if self.is_openrouter {
            req_builder = req_builder
                .header("HTTP-Referer", "https://github.com/ecliven-code")
                .header("X-Title", "Ecliven Code");
        }

        let resp = req_builder.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(AiError::ApiStatus {
                status: status.as_u16(),
                body,
            });
        }

        if !streaming {
            let body: Value = resp
                .json()
                .await
                .map_err(|e| AiError::Parse(e.to_string()))?;
            return parse_non_streaming_response(&body);
        }

        let mut content = String::new();
        let mut partial_tools: HashMap<usize, PartialOpenAiToolCall> = HashMap::new();
        let mut usage: Option<TokenUsage> = None;
        let mut stop_reason: Option<String> = None;

        let mut buffer = String::new();
        let mut byte_stream = resp.bytes_stream();

        while let Some(chunk_res) = byte_stream.next().await {
            let chunk = chunk_res?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(newline_pos) = buffer.find('\n') {
                let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
                buffer.drain(..=newline_pos);

                let Some(data) = line.strip_prefix("data: ") else {
                    continue;
                };
                let trimmed = data.trim();
                if trimmed.is_empty() || trimmed == "[DONE]" {
                    continue;
                }

                let Ok(event) = serde_json::from_str::<Value>(trimmed) else {
                    continue;
                };

                if let Some(u) = event.get("usage").and_then(usage_from) {
                    usage = Some(u);
                }

                let Some(choice) = event.pointer("/choices/0") else {
                    continue;
                };

                if let Some(fr) = choice.get("finish_reason").and_then(Value::as_str) {
                    stop_reason = Some(fr.to_string());
                }

                let Some(delta) = choice.get("delta") else {
                    continue;
                };

                if let Some(text) = delta.get("content").and_then(Value::as_str) {
                    content.push_str(text);
                    if let Some(ref tx) = delta_tx {
                        let _ = tx.send(text.to_string());
                    }
                }

                if let Some(tcs) = delta.get("tool_calls").and_then(Value::as_array) {
                    for tc in tcs {
                        let index = tc.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                        let entry = partial_tools.entry(index).or_default();
                        if let Some(id) = tc.get("id").and_then(Value::as_str) {
                            entry.id = id.to_string();
                        }
                        if let Some(name) = tc.pointer("/function/name").and_then(Value::as_str) {
                            entry.name.push_str(name);
                        }
                        if let Some(args) = tc.pointer("/function/arguments").and_then(Value::as_str) {
                            entry.arguments_json.push_str(args);
                        }
                    }
                }
            }
        }

        let mut indices: Vec<usize> = partial_tools.keys().copied().collect();
        indices.sort_unstable();

        let tool_calls = indices
            .into_iter()
            .filter_map(|i| partial_tools.remove(&i))
            .map(|pt| {
                let arguments = serde_json::from_str(&pt.arguments_json).unwrap_or_else(|_| json!({}));
                ToolCall {
                    id: if pt.id.is_empty() {
                        format!("call_{}", pt.name)
                    } else {
                        pt.id
                    },
                    name: pt.name,
                    arguments,
                }
            })
            .collect();

        Ok(CompletionResponse {
            content,
            tool_calls,
            usage,
            stop_reason,
            external_tools: Vec::new(),
        })
    }
}

fn parse_non_streaming_response(body: &Value) -> Result<CompletionResponse, AiError> {
    let choice = body
        .pointer("/choices/0")
        .ok_or_else(|| AiError::Parse("Missing choices[0] in OpenAI response".into()))?;
    let message = choice
        .get("message")
        .ok_or_else(|| AiError::Parse("Missing message in choices[0]".into()))?;

    let content = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    let mut tool_calls = Vec::new();
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let args_raw = call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let arguments = serde_json::from_str(args_raw).unwrap_or_else(|_| json!({}));
            tool_calls.push(ToolCall {
                id,
                name,
                arguments,
            });
        }
    }

    let usage = body.get("usage").and_then(usage_from);

    let stop_reason = choice
        .get("finish_reason")
        .and_then(Value::as_str)
        .map(ToString::to_string);

    Ok(CompletionResponse {
        content,
        tool_calls,
        usage,
        stop_reason,
        external_tools: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_tool_call_response() {
        let raw = json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": {
                            "name": "grep_content",
                            "arguments": "{\"pattern\":\"fn main\"}"
                        }
                    }]
                }
            }],
            "usage": { "prompt_tokens": 50, "completion_tokens": 18 }
        });

        let parsed = parse_non_streaming_response(&raw).unwrap();
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].id, "call_abc");
        assert_eq!(parsed.tool_calls[0].arguments["pattern"], "fn main");
        assert_eq!(parsed.usage.unwrap().output_tokens, 18);
    }

    fn cache_request() -> CompletionRequest {
        CompletionRequest {
            system_prompt: "system rules".into(),
            messages: vec![Message::user("hello")],
            tools: Vec::new(),
            max_tokens: 64,
            temperature: 0.0,
        }
    }

    #[test]
    fn openai_sends_stable_cache_key_and_stream_usage() {
        let p = OpenAiCompatibleProvider::new("openai", "k".into(), "https://example.com".into(), "test/model".into(), false);
        let a = p.build_payload(&cache_request(), true);
        let b = p.build_payload(&cache_request(), false);
        assert!(a["prompt_cache_key"].as_str().unwrap().starts_with("ecliven-"));
        assert_eq!(a["prompt_cache_key"], b["prompt_cache_key"]);
        assert_eq!(a["stream_options"]["include_usage"], true);
        assert!(b.get("stream_options").is_none());
        let off = p.with_prompt_cache(PromptCache::Off).build_payload(&cache_request(), false);
        assert!(off.get("prompt_cache_key").is_none());
    }

    #[test]
    fn openrouter_adds_breakpoints_only_for_models_that_need_them() {
        let claude = OpenAiCompatibleProvider::new("openrouter", "k".into(), "https://example.com".into(), "anthropic/test-model".into(), true);
        let body = claude.build_payload(&cache_request(), false);
        assert!(body.get("prompt_cache_key").is_none());
        assert_eq!(body["messages"][0]["content"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(body["messages"][1]["content"][0]["text"], "hello");
        let other = OpenAiCompatibleProvider::new("openrouter", "k".into(), "https://example.com".into(), "test/model".into(), true);
        assert!(!other.build_payload(&cache_request(), false).to_string().contains("cache_control"));
    }

    #[test]
    fn usage_reads_cached_tokens() {
        let u = usage_from(&json!({
            "prompt_tokens": 1200, "completion_tokens": 30,
            "prompt_tokens_details": { "cached_tokens": 1024 }
        }))
        .unwrap();
        assert_eq!((u.input_tokens, u.cache_read_tokens, u.output_tokens), (1200, 1024, 30));
    }

    #[test]
    fn user_images_become_image_url_parts() {
        let mut msg = Message::user("what is this");
        msg.images.push(crate::ai::ImageInput { name: "a.png".into(), media_type: "image/png".into(), data: "QUJD".into() });
        let v = convert_message(&msg);
        assert_eq!(v["content"][0]["text"], "what is this");
        assert_eq!(v["content"][1]["image_url"]["url"], "data:image/png;base64,QUJD");
    }

}
