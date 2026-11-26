use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::ai::{
    AiError, CompletionRequest, CompletionResponse, LlmProvider, Message, Role, TokenUsage,
    ToolCall,
};

pub struct OllamaProvider {
    client: Client,
    base_url: String,
    model: String,
}

impl OllamaProvider {
    pub fn new(base_url: String, model: String) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            model,
        }
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

        let mut payload = json!({
            "model": self.model,
            "messages": messages,
            "stream": stream,
            "options": {
                "temperature": request.temperature,
                "num_predict": request.max_tokens,
            }
        });

        if !tools.is_empty() {
            payload["tools"] = Value::Array(tools);
        }

        payload
    }
}

fn convert_message(msg: &Message) -> Value {
    match msg.role {
        Role::System => json!({ "role": "system", "content": msg.content }),
        Role::User => json!({ "role": "user", "content": msg.content }),
        Role::Assistant => {
            let mut obj = json!({
                "role": "assistant",
                "content": msg.content,
            });
            if !msg.tool_calls.is_empty() {
                let calls: Vec<Value> = msg
                    .tool_calls
                    .iter()
                    .map(|tc| {
                        json!({
                            "function": {
                                "name": tc.name,
                                "arguments": tc.arguments,
                            }
                        })
                    })
                    .collect();
                obj["tool_calls"] = Value::Array(calls);
            }
            obj
        }
        Role::Tool => json!({
            "role": "tool",
            "content": msg.content,
        }),
    }
}

fn extract_ollama_tool_calls(message: &Value, counter: &mut usize) -> Vec<ToolCall> {
    let mut out = Vec::new();
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            *counter += 1;
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let arguments = call
                .pointer("/function/arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            out.push(ToolCall {
                id: format!("ollama_call_{counter}"),
                name,
                arguments,
            });
        }
    }
    out
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn name(&self) -> &'static str {
        "ollama"
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
        let url = format!("{}/api/chat", self.base_url);
        let payload = self.build_payload(request, streaming);

        let resp = self
            .client
            .post(&url)
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

        let mut content = String::new();
        let mut tool_calls = Vec::new();
        let mut usage: Option<TokenUsage> = None;
        let mut stop_reason: Option<String> = None;
        let mut call_counter = 0usize;

        if !streaming {
            let body: Value = resp
                .json()
                .await
                .map_err(|e| AiError::Parse(e.to_string()))?;
            if let Some(msg) = body.get("message") {
                if let Some(text) = msg.get("content").and_then(Value::as_str) {
                    content.push_str(text);
                }
                tool_calls.extend(extract_ollama_tool_calls(msg, &mut call_counter));
            }
            usage = Some(TokenUsage {
                input_tokens: body.get("prompt_eval_count").and_then(Value::as_u64).unwrap_or(0) as u32,
                output_tokens: body.get("eval_count").and_then(Value::as_u64).unwrap_or(0) as u32,
            });
            stop_reason = body
                .get("done_reason")
                .and_then(Value::as_str)
                .map(ToString::to_string);

            return Ok(CompletionResponse {
                content,
                tool_calls,
                usage,
                stop_reason,
                external_tools: Vec::new(),
            });
        }

        let mut buffer = String::new();
        let mut byte_stream = resp.bytes_stream();

        while let Some(chunk_res) = byte_stream.next().await {
            let chunk = chunk_res?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(newline_pos) = buffer.find('\n') {
                let line = buffer[..newline_pos].trim().to_string();
                buffer.drain(..=newline_pos);

                if line.is_empty() {
                    continue;
                }
                let Ok(event) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };

                if let Some(msg) = event.get("message") {
                    if let Some(text) = msg.get("content").and_then(Value::as_str) {
                        if !text.is_empty() {
                            content.push_str(text);
                            if let Some(ref tx) = delta_tx {
                                let _ = tx.send(text.to_string());
                            }
                        }
                    }
                    tool_calls.extend(extract_ollama_tool_calls(msg, &mut call_counter));
                }

                if event.get("done").and_then(Value::as_bool) == Some(true) {
                    usage = Some(TokenUsage {
                        input_tokens: event
                            .get("prompt_eval_count")
                            .and_then(Value::as_u64)
                            .unwrap_or(0) as u32,
                        output_tokens: event
                            .get("eval_count")
                            .and_then(Value::as_u64)
                            .unwrap_or(0) as u32,
                    });
                    stop_reason = event
                        .get("done_reason")
                        .and_then(Value::as_str)
                        .map(ToString::to_string);
                }
            }
        }

        Ok(CompletionResponse {
            content,
            tool_calls,
            usage,
            stop_reason,
            external_tools: Vec::new(),
        })
    }
}
