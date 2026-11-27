pub mod checkpoint;
pub mod compact;
pub mod instructions;
pub mod session;
pub mod store;
pub mod system_prompt;

use std::sync::Arc;

use tokio::sync::{mpsc, oneshot};

use crate::agent::compact::{compact_session, estimate_tokens, CompactionPolicy};
use crate::agent::session::Session;
use crate::agent::system_prompt::build_system_prompt;
use crate::ai::{CompletionRequest, LlmProvider, TokenUsage};
use crate::config::ApprovalMode;
use crate::tools::{ToolContext, ToolOutput, ToolPreview, ToolRegistry, ToolRisk};

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
    /// Informational message (e.g. context compaction).
    Notice(String),
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

const SAVER_MAX_TOKENS: u32 = 2048;
const SAVER_TOOL_OUTPUT_CHARS: usize = 4_000;
const SAVER_DIRECTIVE: &str = "\n\nSAVER MODE: minimize tokens and cost. Answer tersely without preamble or recap. \
Read only the files and line ranges you need (prefer grep_content over reading whole files), never re-read a file you already have, \
batch edits with apply_patch, and stop as soon as the task is done.";

pub struct AgentEngine {
    provider: Arc<dyn LlmProvider>,
    tools: Arc<ToolRegistry>,
    tool_ctx: ToolContext,
    options: AgentOptions,
    system_prompt: String,
    compaction: Option<CompactionPolicy>,
    saver: bool,
    knowledge: Option<crate::knowledge::BriefOptions>,
}

const KNOWLEDGE_DIRECTIVE: &str = "\n\nProject knowledge: each user message may end with a <project_context> brief (active task, git state, memories with provenance, index hints). \
Prefer code_search, find_symbol and read_symbol over reading whole files; use find_references and call_graph to follow usages, and impact before changing shared code to find dependents and affected tests; memory_search to recall conventions, update_plan for multi-step tasks, and memory_save only for durable verified facts (cite source files). \
Index hints and stale memories can be outdated: read the current file before editing it.";

const WORKFLOW_DIRECTIVE: &str = "\n\nWORKFLOW for every request: 1) recall applicable rules and context (brief, memory_search); 2) inspect the current files involved; \
3) for multi-step work, record a short plan with update_plan; 4) apply controlled changes (apply_patch/edit_file); 5) run the authorized checks (build/tests) with run_command; \
6) review the diff (git diff) and results; 7) report briefly to the user; 8) save only useful durable facts with memory_save and mark the plan done.";

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
            compaction: None,
            saver: false,
            knowledge: None,
        }
    }

    /// Injects a budgeted project brief per turn and records task history afterwards.
    pub fn with_knowledge(mut self, brief: crate::knowledge::BriefOptions, workflow: bool) -> Self {
        self.knowledge = Some(brief);
        self.system_prompt.push_str(KNOWLEDGE_DIRECTIVE);
        if workflow {
            self.system_prompt.push_str(WORKFLOW_DIRECTIVE);
        }
        self
    }

    /// Saver mode: shorter answers, capped tool output kept in history.
    pub fn with_saver_mode(mut self, enabled: bool) -> Self {
        if enabled {
            self.saver = true;
            self.options.max_tokens = self.options.max_tokens.min(SAVER_MAX_TOKENS);
            self.system_prompt.push_str(SAVER_DIRECTIVE);
        }
        self
    }

    /// Enables automatic compaction when the estimated context reaches `threshold_percent`.
    pub fn with_compaction(mut self, context_window: u32, threshold_percent: u8) -> Self {
        self.compaction = Some(CompactionPolicy {
            context_window,
            threshold_percent,
        });
        self
    }

    pub fn system_prompt(&self) -> &str {
        &self.system_prompt
    }

    async fn maybe_compact(
        &self,
        session: &mut Session,
        event_tx: &mpsc::UnboundedSender<AgentEvent>,
    ) {
        let Some(policy) = self.compaction else {
            return;
        };
        let used = estimate_tokens(&self.system_prompt, session.messages())
            .max(session.last_prompt_tokens());
        if !policy.should_compact(used) {
            return;
        }
        match compact_session(
            &self.provider,
            session,
            &self.system_prompt,
            policy.context_window,
            self.options.max_tokens,
        )
        .await
        {
            Ok(r) => {
                let _ = event_tx.send(AgentEvent::Notice(format!(
                    "Context compacted automatically: ~{} -> ~{} tokens.",
                    r.before_tokens, r.after_tokens
                )));
            }
            Err(err) => {
                let _ = event_tx.send(AgentEvent::Notice(format!(
                    "Automatic context compaction failed: {err}"
                )));
            }
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
        let Some(opts) = self.knowledge else {
            return self.run_turn_inner(session, user_input, event_tx).await;
        };
        let ws = self.tool_ctx.workspace_root.clone();
        let prompt = user_input.clone();
        let brief = tokio::task::spawn_blocking(move || {
            let mut project = crate::knowledge::KnowledgeStore::open(&ws)?;
            let user = crate::knowledge::KnowledgeStore::open_user().ok();
            crate::knowledge::build_brief(&mut project, user.as_ref(), &prompt, opts)
        })
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r);
        let input = match brief {
            Ok(Some(b)) => format!("{user_input}\n\n{b}"),
            Ok(None) => user_input.clone(),
            Err(e) => {
                let _ = event_tx.send(AgentEvent::Notice(format!("Project knowledge unavailable: {e}")));
                user_input.clone()
            }
        };
        let before = session.messages().len();
        self.run_turn_inner(session, input, event_tx).await;

        let root = self.tool_ctx.workspace_root.clone();
        let files: Vec<String> = session
            .checkpoints()
            .last()
            .map(|cp| {
                cp.paths()
                    .map(|p| crate::knowledge::indexer::rel_path(&root, p))
                    .collect()
            })
            .unwrap_or_default();
        let answer = session.messages()[before.min(session.messages().len())..]
            .iter()
            .rev()
            .find(|m| m.role == crate::ai::Role::Assistant && !m.content.trim().is_empty())
            .map(|m| m.content.clone());
        let ok = answer.is_some();
        let summary = answer.unwrap_or_else(|| "(no answer: turn failed or was interrupted)".into());
        let ws = self.tool_ctx.workspace_root.clone();
        let _ = tokio::task::spawn_blocking(move || -> Result<(), String> {
            let mut store = crate::knowledge::KnowledgeStore::open(&ws)?;
            store.record_turn(&user_input, &files, &summary, ok)?;
            if !files.is_empty() {
                store.refresh()?;
            }
            Ok(())
        })
        .await;
    }

    async fn run_turn_inner(
        &self,
        session: &mut Session,
        user_input: String,
        event_tx: mpsc::UnboundedSender<AgentEvent>,
    ) {
        let label: String = user_input
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(60)
            .collect();
        session.begin_checkpoint(label);
        session.push_user(user_input);

        for _iteration in 0..self.options.max_tool_iterations {
            self.maybe_compact(session, &event_tx).await;
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

            for t in &response.external_tools {
                let tool_name = format!("external:{}", t.name);
                let _ = event_tx.send(AgentEvent::ToolStarted {
                    tool_name: tool_name.clone(),
                    summary: t.input.clone(),
                });
                let output = if t.ok {
                    ToolOutput::ok(t.output.clone())
                } else {
                    ToolOutput::err(t.output.clone())
                };
                let _ = event_tx.send(AgentEvent::ToolCompleted { tool_name, output });
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

                let decision = crate::tools::policy::evaluate(
                    &call.name,
                    tool.risk_level(),
                    &self.tool_ctx,
                    self.options.approval_mode,
                );
                if let crate::tools::policy::Decision::Deny(reason) = &decision {
                    let msg = format!("Blocked by security policy: {reason}");
                    session.push_tool_result(&call.id, &call.name, msg.clone());
                    let _ = event_tx.send(AgentEvent::ToolCompleted {
                        tool_name: call.name,
                        output: ToolOutput::err(msg),
                    });
                    continue;
                }
                if let crate::tools::policy::Decision::Ask { note } = decision {
                    let mut preview = preview.clone();
                    if let Some(note) = note {
                        preview.summary = format!("{} [{note}]", preview.summary);
                    }
                    let (resp_tx, resp_rx) = oneshot::channel();
                    let _ = event_tx.send(AgentEvent::ToolApprovalRequested {
                        call_id: call.id.clone(),
                        tool_name: call.name.clone(),
                        preview,
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

                if tool.risk_level() == ToolRisk::FileWrite {
                    session
                        .capture_before_write(&tool.touched_paths(&self.tool_ctx, &call.arguments));
                }

                let output = match tool.execute(&self.tool_ctx, &call.arguments).await {
                    Ok(out) => out,
                    Err(err) => ToolOutput::err(format!("Tool error: {err}")),
                };

                let kept = if self.saver && output.content.len() > SAVER_TOOL_OUTPUT_CHARS {
                    format!(
                        "{}\n... [truncated in saver mode]",
                        crate::tools::truncate_str(&output.content, SAVER_TOOL_OUTPUT_CHARS)
                    )
                } else {
                    output.content.clone()
                };
                session.push_tool_result(&call.id, &call.name, kept);
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
                        ..Default::default()
                    }),
                    stop_reason: Some("tool_use".into()),
                    external_tools: Vec::new(),
                },
                CompletionResponse {
                    content: "The file says: rust rocks".into(),
                    tool_calls: Vec::new(),
                    usage: Some(TokenUsage {
                        input_tokens: 20,
                        output_tokens: 8,
                        ..Default::default()
                    }),
                    stop_reason: Some("end_turn".into()),
                    external_tools: Vec::new(),
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
        assert_eq!(
            session.messages().last().unwrap().content,
            "The file says: rust rocks"
        );
    }
}
