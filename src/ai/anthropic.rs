use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::ai::{
    AiError, CompletionRequest, CompletionResponse, LlmProvider, Message, Role, TokenUsage,
    ToolCall,
};

const ANTHROPIC_API_VERSION: &str = "2023-06-01";

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: String,
    model: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String, base_url: String, model: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
        }
    }

    fn build_payload(&self, request: &CompletionRequest, stream: bool) -> Value {
        let messages = convert_messages(&request.messages);
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect();

        let mut payload = json!({
            "model": self.model,
            "max_tokens": request.max_tokens,
            "temperature": request.temperature,
            "system": request.system_prompt,
            "messages": messages,
            "stream": stream,
        });

        if !tools.is_empty() {
            payload["tools"] = Value::Array(tools);
        }

        payload
    }
}

fn convert_messages(messages: &[Message]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut idx = 0;

    while idx < messages.len() {
        let msg = &messages[idx];
        match msg.role {
            Role::System => {
                idx += 1;
            }
            Role::User => {
                out.push(json!({
                    "role": "user",
                    "content": msg.content,
                }));
                idx += 1;
            }
            Role::Assistant => {
                let mut blocks: Vec<Value> = Vec::new();
                if !msg.content.is_empty() {
                    blocks.push(json!({
                        "type": "text",
                        "text": msg.content,
                    }));
                }
                for tc in &msg.tool_calls {
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        "input": tc.arguments,
                    }));
                }
                if blocks.is_empty() {
                    blocks.push(json!({ "type": "text", "text": "" }));
                }
                out.push(json!({
                    "role": "assistant",
                    "content": blocks,
                }));
                idx += 1;
            }
            Role::Tool => {
                let mut tool_blocks: Vec<Value> = Vec::new();
                while idx < messages.len() && messages[idx].role == Role::Tool {
                    let tm = &messages[idx];
                    tool_blocks.push(json!({
                        "type": "tool_result",
                        "tool_use_id": tm.tool_call_id.as_deref().unwrap_or("unknown"),
                        "content": tm.content,
                    }));
                    idx += 1;
                }
                out.push(json!({
                    "role": "user",
                    "content": tool_blocks,
                }));
            }
        }
    }

    out
}

#[derive(Default)]
struct PartialToolUse {
    id: String,
    name: String,
    input_json: String,
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &'static str {
        "anthropic"
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
        let url = format!("{}/v1/messages", self.base_url);
        let payload = self.build_payload(request, streaming);

        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .header("content-type", "application/json")
            .json(&payload)
            .send()
            .await?;

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
        let mut partial_tools: HashMap<usize, PartialToolUse> = HashMap::new();
        let mut usage = TokenUsage::default();
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
                if data.trim().is_empty() || data.trim() == "[DONE]" {
                    continue;
                }

                let Ok(event) = serde_json::from_str::<Value>(data) else {
                    continue;
                };

                match event.get("type").and_then(Value::as_str).unwrap_or("") {
                    "message_start" => {
                        if let Some(u) = event.pointer("/message/usage/input_tokens").and_then(Value::as_u64) {
                            usage.input_tokens = u as u32;
                        }
                    }
                    "content_block_start" => {
                        let index = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                        if let Some(block) = event.get("content_block") {
                            if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                                partial_tools.insert(
                                    index,
                                    PartialToolUse {
                                        id: block
                                            .get("id")
                                            .and_then(Value::as_str)
                                            .unwrap_or_default()
                                            .to_string(),
                                        name: block
                                            .get("name")
                                            .and_then(Value::as_str)
                                            .unwrap_or_default()
                                            .to_string(),
                                        input_json: String::new(),
                                    },
                                );
                            }
                        }
                    }
                    "content_block_delta" => {
                        let index = event.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
                        if let Some(delta) = event.get("delta") {
                            match delta.get("type").and_then(Value::as_str).unwrap_or("") {
                                "text_delta" => {
                                    if let Some(text) = delta.get("text").and_then(Value::as_str) {
                                        content.push_str(text);
                                        if let Some(ref tx) = delta_tx {
                                            let _ = tx.send(text.to_string());
                                        }
                                    }
                                }
                                "input_json_delta" => {
                                    if let Some(partial) = delta.get("partial_json").and_then(Value::as_str) {
                                        if let Some(entry) = partial_tools.get_mut(&index) {
                                            entry.input_json.push_str(partial);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    "message_delta" => {
                        if let Some(sr) = event.pointer("/delta/stop_reason").and_then(Value::as_str) {
                            stop_reason = Some(sr.to_string());
                        }
                        if let Some(out_toks) = event.pointer("/usage/output_tokens").and_then(Value::as_u64) {
                            usage.output_tokens = out_toks as u32;
                        }
                    }
                    _ => {}
                }
            }
        }

        let mut indices: Vec<usize> = partial_tools.keys().copied().collect();
        indices.sort_unstable();

        let tool_calls = indices
            .into_iter()
            .filter_map(|i| partial_tools.remove(&i))
            .map(|pt| {
                let arguments = if pt.input_json.trim().is_empty() {
                    json!({})
                } else {
                    serde_json::from_str(&pt.input_json).unwrap_or_else(|_| json!({}))
                };
                ToolCall {
                    id: pt.id,
                    name: pt.name,
                    arguments,
                }
            })
            .collect();

        Ok(CompletionResponse {
            content,
            tool_calls,
            usage: Some(usage),
            stop_reason,
            external_tools: Vec::new(),
        })
    }
}

fn parse_non_streaming_response(body: &Value) -> Result<CompletionResponse, AiError> {
    let mut content = String::new();
    let mut tool_calls = Vec::new();

    if let Some(blocks) = body.get("content").and_then(Value::as_array) {
        for block in blocks {
            match block.get("type").and_then(Value::as_str).unwrap_or("") {
                "text" => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        content.push_str(text);
                    }
                }
                "tool_use" => {
                    tool_calls.push(ToolCall {
                        id: block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        name: block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        arguments: block.get("input").cloned().unwrap_or_else(|| json!({})),
                    });
                }
                _ => {}
            }
        }
    }

    let usage = body.get("usage").map(|u| TokenUsage {
        input_tokens: u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
        output_tokens: u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0) as u32,
    });

    let stop_reason = body
        .get("stop_reason")
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
    fn groups_consecutive_tool_results_into_single_user_turn() {
        let msgs = vec![
            Message::user("Inspect project"),
            Message::assistant(
                "Checking files",
                vec![
                    ToolCall {
                        id: "tc_1".into(),
                        name: "list_dir".into(),
                        arguments: json!({"path": "."}),
                    },
                    ToolCall {
                        id: "tc_2".into(),
                        name: "read_file".into(),
                        arguments: json!({"path": "Cargo.toml"}),
                    },
                ],
            ),
            Message::tool_result("tc_1", "list_dir", "src/\nCargo.toml"),
            Message::tool_result("tc_2", "read_file", "[package]\nname = \"ecliven\""),
        ];

        let converted = convert_messages(&msgs);
        assert_eq!(converted.len(), 3);
        assert_eq!(converted[2]["role"], "user");
        assert_eq!(converted[2]["content"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn parses_non_streaming_anthropic_tool_response() {
        let raw = json!({
            "content": [
                { "type": "text", "text": "Let me inspect that." },
                { "type": "tool_use", "id": "toolu_01", "name": "read_file", "input": { "path": "src/main.rs" } }
            ],
            "stop_reason": "tool_use",
            "usage": { "input_tokens": 120, "output_tokens": 45 }
        });

        let parsed = parse_non_streaming_response(&raw).unwrap();
        assert_eq!(parsed.content, "Let me inspect that.");
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].name, "read_file");
        assert_eq!(parsed.usage.unwrap().input_tokens, 120);
    }
}
