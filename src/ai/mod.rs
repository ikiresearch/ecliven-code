pub mod adaptive;
pub mod anthropic;
pub mod cli_backend;
pub mod cli_events;
pub mod models;
pub mod ollama;
pub mod openai;
pub mod registry;
pub mod router;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::mpsc;

#[derive(Debug, Error)]
pub enum AiError {
    #[error("Missing API key for provider '{provider}'. Set {env_var} in your environment or .env file.")]
    MissingApiKey {
        provider: &'static str,
        env_var: &'static str,
    },

    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Provider returned HTTP {status}: {body}")]
    ApiStatus { status: u16, body: String },

    #[error("CLI '{cli}' failed (exit {code}): {output}")]
    CliFailed {
        cli: String,
        code: i32,
        output: String,
    },

    #[error("Failed to parse provider response: {0}")]
    Parse(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
        }
    }

    pub fn assistant(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            tool_calls,
            tool_call_id: None,
            tool_name: None,
        }
    }

    pub fn tool_result(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        Self {
            role: Role::Tool,
            content: output.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
            tool_name: Some(tool_name.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: u32,
    pub temperature: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenUsage {
    /// All prompt tokens of the request, cached or not (drives the context bar).
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Prompt tokens served from the provider's prompt cache (billed at a discount).
    pub cache_read_tokens: u32,
    /// Prompt tokens written to the prompt cache by this request.
    pub cache_write_tokens: u32,
}

/// Prompt caching: reuse the stable prefix (system prompt, tools, history)
/// across turns. `5m`/`1h` set the cache lifetime where the provider allows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum PromptCache {
    #[serde(rename = "off")]
    Off,
    #[default]
    #[serde(rename = "5m")]
    Short,
    #[serde(rename = "1h")]
    Long,
}

#[derive(Debug, Clone, Default)]
pub struct CompletionResponse {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<TokenUsage>,
    pub stop_reason: Option<String>,
    /// Tools already run by an external CLI backend (informational, not executed again).
    pub external_tools: Vec<ExternalToolCall>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExternalToolCall {
    pub name: String,
    pub input: String,
    pub output: String,
    pub ok: bool,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn model(&self) -> &str;

    async fn complete(
        &self,
        request: &CompletionRequest,
        delta_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<CompletionResponse, AiError>;

    /// Backends that run their own agent in a directory (external CLIs) return a
    /// copy bound to `dir` and `sandbox`; API backends never touch files and return `None`.
    fn rebased(
        &self,
        _dir: &std::path::Path,
        _sandbox: crate::tools::sandbox::SandboxMode,
    ) -> Option<std::sync::Arc<dyn LlmProvider>> {
        None
    }
}
