pub mod session;
pub mod system_prompt;

use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};

use crate::agent::session::Session;
use crate::agent::system_prompt::build_system_prompt;
use crate::ai::{CompletionRequest, LlmProvider, TokenUsage};
use crate::config::ApprovalMode;
use crate::tools::{ToolContext, ToolOutput, ToolPreview, ToolRegistry};

pub enum AgentEvent {
    AssistantDelta(String),
    AssistantFinished {
        content: String,
        usage: Option<TokenUsage>,
    },
    ToolApprovalRequested {
        call_id: String,
        tool_name: String,
        preview: ToolPreview,
        respond_to: oneshot::Sender<bool>,
    },
    ToolStarted {
        tool_name: String,
        summary: String,
    },
    ToolCompleted {
        tool_name: String,
        output: ToolOutput,
    },
    Error(String),
    TurnComplete,
}

#[derive(Debug, Clone)]
pub struct AgentOptions {
    pub max_tokens: u32,
    pub temperature: f32,
    pub max_tool_iterations: usize,
    pub approval_mode: ApprovalMode,
}

pub struct AgentEngine {
    provider: Arc<dyn LlmProvider>,
    tools: Arc<ToolRegistry>,
    tool_ctx: ToolContext,
    options: AgentOptions,
    system_prompt: String,
}

impl AgentEngine {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        tools: Arc<ToolRegistry>,
        tool_ctx: ToolContext,
        options: AgentOptions,
    ) -> Self {
        let system_prompt = build_system_prompt(&tool_ctx.workspace_root);
        Self::with_system_prompt(provider, tools, tool_ctx, options, system_prompt)
    }

    pub fn with_system_prompt(
        provider: Arc<dyn LlmProvider>,
        tools: Arc<ToolRegistry>,
        tool_ctx: ToolContext,
        options: AgentOptions,
        system_prompt: String,
    ) -> Self {
        Self {
            provider,
            tools,
            tool_ctx,
            options,
            system_prompt,
        }
    }

    pub fn set_provider(&mut self, provider: Arc<dyn LlmProvider>) {
        self.provider = provider;
    }

    pub fn set_approval_mode(&mut self, mode: ApprovalMode) {
        self.options.approval_mode = mode;
    }

    pub async fn run_turn(
        &self,
        session: &mut Session,
        user_input: String,
        event_tx: mpsc::UnboundedSender<AgentEvent>,
    ) {
        session.push_user(user_input);

        for _iteration in 0..self.options.max_tool_iterations {
            let request = CompletionRequest {
                system_prompt: self.system_prompt.clone(),
                messages: session.messages().to_vec(),
                tools: self.tools.specs(),
                max_tokens: self.options.max_tokens,
                temperature: self.options.temperature,
            };

            let (delta_tx, mut delta_rx) = mpsc::unbounded_channel::<String>();
            let forward_tx = event_tx.clone();
            let forward_handle = tokio::spawn(async move {
                while let Some(chunk) = delta_rx.recv().await {
                    let _ = forward_tx.send(AgentEvent::AssistantDelta(chunk));
                }
            });

            let response = match self.provider.complete(&request, Some(delta_tx)).await {
                Ok(resp) => {
                    let _ = forward_handle.await;
                    resp
                }
                Err(err) => {
                    let _ = forward_handle.await;
                    let _ = event_tx.send(AgentEvent::Error(err.to_string()));
                    let _ = event_tx.send(AgentEvent::TurnComplete);
                    return;
                }
            };

            if let Some(ref usage) = response.usage {
                session.add_usage(usage);
            }

            session.push_assistant(response.content.clone(), response.tool_calls.clone());

            let _ = event_tx.send(AgentEvent::AssistantFinished {
                content: response.content,
                usage: response.usage,
            });

            if response.tool_calls.is_empty() {
                let _ = event_tx.send(AgentEvent::TurnComplete);
                return;
            }

            for call in response.tool_calls {
                let Some(tool) = self.tools.get(&call.name) else {
                    let err_msg = format!("Unknown tool '{}'", call.name);
                    let output = ToolOutput::err(err_msg.clone());
                    session.push_tool_result(&call.id, &call.name, err_msg);
                    let _ = event_tx.send(AgentEvent::ToolCompleted {
                        tool_name: call.name,
                        output,
                    });
                    continue;
                };

                let preview = match tool.preview(&self.tool_ctx, &call.arguments) {
                    Ok(p) => p,
                    Err(err) => {
                        let msg = format!("Tool preview error: {err}");
                        let output = ToolOutput::err(msg.clone());
                        session.push_tool_result(&call.id, &call.name, msg);
                        let _ = event_tx.send(AgentEvent::ToolCompleted {
                            tool_name: call.name,
                            output,
                        });
                        continue;
                    }
                };

                if tool.risk_level().requires_approval(self.options.approval_mode) {
                    let (resp_tx, resp_rx) = oneshot::channel();
                    let _ = event_tx.send(AgentEvent::ToolApprovalRequested {
                        call_id: call.id.clone(),
                        tool_name: call.name.clone(),
                        preview: preview.clone(),
                        respond_to: resp_tx,
                    });

                    let approved = resp_rx.await.unwrap_or(false);
                    if !approved {
                        let denied = format!(
                            "User denied permission to execute '{}'. Ask for guidance or try a non-destructive alternative.",
                            call.name
                        );
                        let output = ToolOutput::err(denied.clone());
                        session.push_tool_result(&call.id, &call.name, denied);
                        let _ = event_tx.send(AgentEvent::ToolCompleted {
                            tool_name: call.name,
                            output,
                        });
                        continue;
                    }
                }

                let _ = event_tx.send(AgentEvent::ToolStarted {
                    tool_name: call.name.clone(),
                    summary: preview.summary,
                });

                let output = match tool.execute(&self.tool_ctx, &call.arguments).await {
                    Ok(out) => out,
                    Err(err) => ToolOutput::err(format!("Tool error: {err}")),
                };

                session.push_tool_result(&call.id, &call.name, output.content.clone());
                let _ = event_tx.send(AgentEvent::ToolCompleted {
                    tool_name: call.name,
                    output,
                });
            }
        }

        let _ = event_tx.send(AgentEvent::Error(format!(
            "Reached maximum tool iterations ({})",
            self.options.max_tool_iterations
        )));
        let _ = event_tx.send(AgentEvent::TurnComplete);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{AiError, CompletionResponse, ToolCall};
    use async_trait::async_trait;
    use serde_json::json;
    use std::sync::Mutex;
    use tempfile::TempDir;

    struct MockProvider {
        responses: Mutex<Vec<CompletionResponse>>,
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        fn name(&self) -> &'static str {
            "mock"
        }

        fn model(&self) -> &str {
            "mock-1"
        }

        async fn complete(
            &self,
            _request: &CompletionRequest,
            delta_tx: Option<mpsc::UnboundedSender<String>>,
        ) -> Result<CompletionResponse, AiError> {
            let resp = self.responses.lock().unwrap().remove(0);
            if let Some(tx) = delta_tx {
                if !resp.content.is_empty() {
                    let _ = tx.send(resp.content.clone());
                }
            }
            Ok(resp)
        }
    }

    #[tokio::test]
    async fn agent_executes_tool_call_and_returns_final_answer() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("hello.txt"), "rust rocks").unwrap();

        let provider = Arc::new(MockProvider {
            responses: Mutex::new(vec![
                CompletionResponse {
                    content: "Reading hello.txt...".into(),
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        name: "read_file".into(),
                        arguments: json!({ "path": "hello.txt" }),
                    }],
                    usage: Some(TokenUsage {
                        input_tokens: 10,
                        output_tokens: 5,
                    }),
                    stop_reason: Some("tool_use".into()),
                },
                CompletionResponse {
                    content: "The file says: rust rocks".into(),
                    tool_calls: Vec::new(),
                    usage: Some(TokenUsage {
                        input_tokens: 20,
                        output_tokens: 8,
                    }),
                    stop_reason: Some("end_turn".into()),
                },
            ]),
        });

        let engine = AgentEngine::new(
            provider,
            Arc::new(ToolRegistry::default_registry()),
            ToolContext::new(tmp.path().to_path_buf(), 10),
            AgentOptions {
                max_tokens: 512,
                temperature: 0.0,
                max_tool_iterations: 5,
                approval_mode: ApprovalMode::Ask,
            },
        );

        let mut session = Session::new();
        let (tx, mut rx) = mpsc::unbounded_channel();

        engine
            .run_turn(&mut session, "What is in hello.txt?".into(), tx)
            .await;

        let mut completed_tools = 0;
        let mut saw_turn_complete = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                AgentEvent::ToolCompleted { .. } => completed_tools += 1,
                AgentEvent::TurnComplete => saw_turn_complete = true,
                _ => {}
            }
        }

        assert_eq!(completed_tools, 1);
        assert!(saw_turn_complete);
        assert_eq!(session.total_usage().input_tokens, 30);
        assert_eq!(session.total_usage().output_tokens, 13);
        assert_eq!(session.messages().last().unwrap().content, "The file says: rust rocks");
    }
}
