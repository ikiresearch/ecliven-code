pub mod domain;

use std::sync::Arc;

use futures::future::join_all;
use tokio::sync::{mpsc, oneshot};

use crate::agent::session::Session;
use crate::agent::system_prompt::build_system_prompt;
use crate::agent::{AgentEngine, AgentEvent, AgentOptions};
use crate::ai::{LlmProvider, TokenUsage};
use crate::config::ApprovalMode;
use crate::swarm::domain::{SwarmAgentRole, SwarmDomain};
use crate::tools::{ToolContext, ToolOutput, ToolPreview, ToolRegistry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwarmAgentStatus {
    Idle,
    Running { active_tool: Option<String> },
    Done,
    Failed(String),
}

pub enum SwarmEvent {
    SwarmStarted {
        domain: SwarmDomain,
        scope: Option<String>,
    },
    WorkerStatus {
        agent_id: String,
        agent_name: String,
        badge: String,
        status: SwarmAgentStatus,
    },
    WorkerReport {
        agent_id: String,
        agent_name: String,
        badge: String,
        content: String,
        usage: Option<TokenUsage>,
    },
    Synthesizing {
        lead_badge: String,
    },
    SynthesisDelta(String),
    SynthesisFinished {
        lead_badge: String,
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
    SwarmComplete,
}

#[derive(Debug, Clone)]
pub struct SwarmOptions {
    pub max_tokens: u32,
    pub temperature: f32,
    pub worker_max_iterations: usize,
    pub lead_max_iterations: usize,
    pub approval_mode: ApprovalMode,
    pub target_scope: Option<String>,
}

impl Default for SwarmOptions {
    fn default() -> Self {
        Self {
            max_tokens: 4096,
            temperature: 0.2,
            worker_max_iterations: 8,
            lead_max_iterations: 10,
            approval_mode: ApprovalMode::Ask,
            target_scope: None,
        }
    }
}

pub struct SwarmEngine {
    domain: SwarmDomain,
    provider: Arc<dyn LlmProvider>,
    worker_providers: Vec<Arc<dyn LlmProvider>>,
    lead_provider: Option<Arc<dyn LlmProvider>>,
    full_tools: Arc<ToolRegistry>,
    read_only_tools: Arc<ToolRegistry>,
    tool_ctx: ToolContext,
    options: SwarmOptions,
}

struct WorkerOutcome {
    role: SwarmAgentRole,
    report: String,
    usage: TokenUsage,
    error: Option<String>,
}

impl SwarmEngine {
    pub fn new(
        domain: SwarmDomain,
        provider: Arc<dyn LlmProvider>,
        full_tools: Arc<ToolRegistry>,
        tool_ctx: ToolContext,
        options: SwarmOptions,
    ) -> Self {
        Self {
            domain,
            provider,
            worker_providers: Vec::new(),
            lead_provider: None,
            full_tools,
            read_only_tools: Arc::new(ToolRegistry::read_only_registry()),
            tool_ctx,
            options,
        }
    }

    /// Uses different models (possibly from different labs) per agent:
    /// workers are assigned round-robin, the lead falls back to the base provider.
    pub fn with_providers(
        mut self,
        workers: Vec<Arc<dyn LlmProvider>>,
        lead: Option<Arc<dyn LlmProvider>>,
    ) -> Self {
        self.worker_providers = workers;
        self.lead_provider = lead;
        self
    }

    fn worker_provider(&self, idx: usize) -> Arc<dyn LlmProvider> {
        if self.worker_providers.is_empty() {
            Arc::clone(&self.provider)
        } else {
            Arc::clone(&self.worker_providers[idx % self.worker_providers.len()])
        }
    }

    pub fn domain(&self) -> SwarmDomain {
        self.domain
    }

    pub async fn run_turn(
        &self,
        session: &mut Session,
        user_input: String,
        event_tx: mpsc::UnboundedSender<SwarmEvent>,
    ) {
        let validated_scope = match self.options.target_scope.as_deref() {
            Some(raw) if !raw.trim().is_empty() => {
                let trimmed = raw.trim();
                match self.tool_ctx.resolve_path(trimmed) {
                    Ok(resolved) if resolved.exists() => Some(trimmed.to_string()),
                    Ok(_) => {
                        let _ = event_tx.send(SwarmEvent::Error(format!(
                            "Target scope '{trimmed}' does not exist in workspace."
                        )));
                        let _ = event_tx.send(SwarmEvent::SwarmComplete);
                        return;
                    }
                    Err(err) => {
                        let _ = event_tx.send(SwarmEvent::Error(format!(
                            "Invalid target scope '{trimmed}': {err}"
                        )));
                        let _ = event_tx.send(SwarmEvent::SwarmComplete);
                        return;
                    }
                }
            }
            _ => None,
        };

        let _ = event_tx.send(SwarmEvent::SwarmStarted {
            domain: self.domain,
            scope: validated_scope.clone(),
        });

        let base_prompt = build_system_prompt(&self.tool_ctx.workspace_root);
        let specialists = self.domain.specialists();
        let prior_context = summarize_recent_history(session);

        let mixed_models = !self.worker_providers.is_empty();
        let worker_futures = specialists.iter().copied().enumerate().map(|(idx, role)| {
            let provider = self.worker_provider(idx);
            let tools = Arc::clone(&self.read_only_tools);
            let tool_ctx = self.tool_ctx.clone();
            let options = self.options.clone();
            let base_prompt = base_prompt.clone();
            let user_input = user_input.clone();
            let prior_context = prior_context.clone();
            let scope = validated_scope.clone();
            let event_tx = event_tx.clone();

            tokio::spawn(async move {
                run_worker(
                    role,
                    mixed_models,
                    provider,
                    tools,
                    tool_ctx,
                    options,
                    base_prompt,
                    prior_context,
                    scope,
                    user_input,
                    event_tx,
                )
                .await
            })
        });

        let join_results = join_all(worker_futures).await;
        let mut outcomes: Vec<WorkerOutcome> = Vec::with_capacity(specialists.len());

        for (idx, res) in join_results.into_iter().enumerate() {
            match res {
                Ok(outcome) => {
                    session.add_usage(&outcome.usage);
                    outcomes.push(outcome);
                }
                Err(join_err) => {
                    let role = specialists[idx];
                    let msg = format!("Worker task panicked: {join_err}");
                    let _ = event_tx.send(SwarmEvent::WorkerStatus {
                        agent_id: role.id.to_string(),
                        agent_name: role.name.to_string(),
                        badge: role.badge.to_string(),
                        status: SwarmAgentStatus::Failed(msg.clone()),
                    });
                    outcomes.push(WorkerOutcome {
                        role,
                        report: String::new(),
                        usage: TokenUsage::default(),
                        error: Some(msg),
                    });
                }
            }
        }

        let lead_badge = self.domain.lead_badge().to_string();
        let _ = event_tx.send(SwarmEvent::Synthesizing {
            lead_badge: lead_badge.clone(),
        });

        let synthesis_prompt = build_synthesis_prompt(
            self.domain,
            validated_scope.as_deref(),
            &user_input,
            &outcomes,
        );
        let lead_system_prompt = format!("{base_prompt}\n\n{}", self.domain.lead_directive());

        let lead_engine = AgentEngine::with_system_prompt(
            Arc::clone(self.lead_provider.as_ref().unwrap_or(&self.provider)),
            Arc::clone(&self.full_tools),
            self.tool_ctx.clone(),
            AgentOptions {
                max_tokens: self.options.max_tokens,
                temperature: self.options.temperature,
                max_tool_iterations: self.options.lead_max_iterations,
                approval_mode: self.options.approval_mode,
            },
            lead_system_prompt,
        );

        let (lead_tx, mut lead_rx) = mpsc::unbounded_channel::<AgentEvent>();
        let forward_tx = event_tx.clone();
        let badge_clone = lead_badge.clone();

        let forward_handle = tokio::spawn(async move {
            while let Some(ev) = lead_rx.recv().await {
                match ev {
                    AgentEvent::AssistantDelta(chunk) => {
                        let _ = forward_tx.send(SwarmEvent::SynthesisDelta(chunk));
                    }
                    AgentEvent::AssistantFinished { content, usage } => {
                        let _ = forward_tx.send(SwarmEvent::SynthesisFinished {
                            lead_badge: badge_clone.clone(),
                            content,
                            usage,
                        });
                    }
                    AgentEvent::ToolApprovalRequested {
                        call_id,
                        tool_name,
                        preview,
                        respond_to,
                    } => {
                        let _ = forward_tx.send(SwarmEvent::ToolApprovalRequested {
                            call_id,
                            tool_name,
                            preview,
                            respond_to,
                        });
                    }
                    AgentEvent::ToolStarted { tool_name, summary } => {
                        let _ = forward_tx.send(SwarmEvent::ToolStarted { tool_name, summary });
                    }
                    AgentEvent::ToolCompleted { tool_name, output } => {
                        let _ = forward_tx.send(SwarmEvent::ToolCompleted { tool_name, output });
                    }
                    AgentEvent::Error(err) => {
                        let _ = forward_tx.send(SwarmEvent::Error(err));
                    }
                    AgentEvent::Notice(_) => {}
                    AgentEvent::TurnComplete => break,
                }
            }
        });

        lead_engine
            .run_turn(session, synthesis_prompt, lead_tx)
            .await;
        let _ = forward_handle.await;

        let _ = event_tx.send(SwarmEvent::SwarmComplete);
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_worker(
    role: SwarmAgentRole,
    show_model: bool,
    provider: Arc<dyn LlmProvider>,
    read_only_tools: Arc<ToolRegistry>,
    tool_ctx: ToolContext,
    options: SwarmOptions,
    base_prompt: String,
    prior_context: Option<String>,
    target_scope: Option<String>,
    user_input: String,
    event_tx: mpsc::UnboundedSender<SwarmEvent>,
) -> WorkerOutcome {
    let agent_name = if show_model {
        format!("{} [{}]", role.name, provider.model())
    } else {
        role.name.to_string()
    };
    let _ = event_tx.send(SwarmEvent::WorkerStatus {
        agent_id: role.id.to_string(),
        agent_name: agent_name.clone(),
        badge: role.badge.to_string(),
        status: SwarmAgentStatus::Running { active_tool: None },
    });

    let system_prompt = format!("{base_prompt}\n\n{}", role.system_directive);
    let engine = AgentEngine::with_system_prompt(
        provider,
        read_only_tools,
        tool_ctx,
        AgentOptions {
            max_tokens: options.max_tokens,
            temperature: options.temperature,
            max_tool_iterations: options.worker_max_iterations,
            approval_mode: ApprovalMode::Ask,
        },
        system_prompt,
    );

    let mut prompt = String::new();
    if let Some(scope) = target_scope {
        prompt.push_str(&format!(
            "Target folder/file scope inside workspace: `{scope}`. Focus your inspection and analysis on `{scope}`.\n\n"
        ));
    }
    if let Some(ctx) = prior_context {
        prompt.push_str(&format!("Recent session context:\n{ctx}\n\n"));
    }
    prompt.push_str(&format!("Task:\n{user_input}"));

    let mut worker_session = Session::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();

    let status_tx = event_tx.clone();
    let collect_name = agent_name.clone();
    let collect_handle = tokio::spawn(async move {
        let agent_name = collect_name;
        let mut last_content = String::new();
        let mut last_error: Option<String> = None;

        while let Some(ev) = rx.recv().await {
            match ev {
                AgentEvent::ToolStarted { tool_name, .. } => {
                    let _ = status_tx.send(SwarmEvent::WorkerStatus {
                        agent_id: role.id.to_string(),
                        agent_name: agent_name.clone(),
                        badge: role.badge.to_string(),
                        status: SwarmAgentStatus::Running {
                            active_tool: Some(tool_name),
                        },
                    });
                }
                AgentEvent::AssistantFinished { content, .. } => {
                    if !content.trim().is_empty() {
                        last_content = content;
                    }
                }
                AgentEvent::Error(err) => {
                    last_error = Some(err);
                }
                AgentEvent::TurnComplete => break,
                _ => {}
            }
        }

        (last_content, last_error)
    });

    engine.run_turn(&mut worker_session, prompt, tx).await;
    let (report, error) = collect_handle.await.unwrap_or_default();
    let usage = worker_session.total_usage().clone();

    if let Some(ref err) = error {
        let _ = event_tx.send(SwarmEvent::WorkerStatus {
            agent_id: role.id.to_string(),
            agent_name: agent_name.clone(),
            badge: role.badge.to_string(),
            status: SwarmAgentStatus::Failed(err.clone()),
        });
    } else {
        let _ = event_tx.send(SwarmEvent::WorkerStatus {
            agent_id: role.id.to_string(),
            agent_name: agent_name.clone(),
            badge: role.badge.to_string(),
            status: SwarmAgentStatus::Done,
        });
        let _ = event_tx.send(SwarmEvent::WorkerReport {
            agent_id: role.id.to_string(),
            agent_name: agent_name.clone(),
            badge: role.badge.to_string(),
            content: report.clone(),
            usage: Some(usage.clone()),
        });
    }

    WorkerOutcome {
        role,
        report,
        usage,
        error,
    }
}

fn summarize_recent_history(session: &Session) -> Option<String> {
    let msgs = session.messages();
    if msgs.is_empty() {
        return None;
    }

    let recent: Vec<String> = msgs
        .iter()
        .rev()
        .filter(|m| !m.content.trim().is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|m| {
            let preview = if m.content.chars().count() > 400 {
                let head: String = m.content.chars().take(400).collect();
                format!("{head}...")
            } else {
                m.content.clone()
            };
            format!("[{:?}] {preview}", m.role)
        })
        .collect();

    if recent.is_empty() {
        None
    } else {
        Some(recent.join("\n"))
    }
}

fn build_synthesis_prompt(
    domain: SwarmDomain,
    target_scope: Option<&str>,
    user_input: &str,
    outcomes: &[WorkerOutcome],
) -> String {
    let mut out = format!("Domain: {}\n", domain.display_name());
    if let Some(scope) = target_scope {
        out.push_str(&format!("Target folder/file scope: `{scope}`\n"));
    }
    out.push_str(&format!("Original user request:\n{user_input}\n\n"));

    if matches!(domain, SwarmDomain::Code) {
        out.push_str("=== Candidate Solutions (Same Task, 3 Independent Coders) ===\n");
    } else {
        out.push_str("=== Specialist Security Reports ===\n");
    }

    for outcome in outcomes {
        out.push_str(&format!(
            "\n--- [{}] {} ---\n",
            outcome.role.badge, outcome.role.name
        ));
        if let Some(ref err) = outcome.error {
            out.push_str(&format!("(Agent failed: {err})\n"));
        } else if outcome.report.trim().is_empty() {
            out.push_str("(No output reported)\n");
        } else {
            out.push_str(&outcome.report);
            out.push('\n');
        }
    }

    match domain {
        SwarmDomain::Code => {
            out.push_str(
                "\n=== Master Agent Evaluation & Combination Rubric ===\n\
                 1. Compare [CLEAN], [PERF], and [ROBUST] on correctness, clarity, performance, and edge cases.\n\
                 2. Cherry-pick and combine the strongest code from each candidate into ONE optimal final implementation.\n\
                 3. Apply the combined changes to the workspace if the user asked for edits, and explain briefly what was taken from each candidate.",
            );
        }
        SwarmDomain::Security(_) => {
            out.push_str(
                "\n=== Security Lead Synthesis Instructions ===\n\
                 Consolidate the specialist reports into a prioritized defensive report (CRITICAL / HIGH / MEDIUM / LOW) with exact file:line citations and concrete remediations.",
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{AiError, CompletionRequest, CompletionResponse};
    use async_trait::async_trait;
    use tempfile::TempDir;

    struct EchoRoleProvider;

    #[async_trait]
    impl LlmProvider for EchoRoleProvider {
        fn name(&self) -> &'static str {
            "mock-swarm"
        }

        fn model(&self) -> &str {
            "mock-1"
        }

        async fn complete(
            &self,
            request: &CompletionRequest,
            delta_tx: Option<mpsc::UnboundedSender<String>>,
        ) -> Result<CompletionResponse, AiError> {
            let content = if request.system_prompt.contains("Defense Mode")
                && request
                    .system_prompt
                    .contains("Defensive Cybersecurity Lead")
            {
                "Defense Lead prioritized vulnerability report.".to_string()
            } else if request.system_prompt.contains("Threat Model")
                && request
                    .system_prompt
                    .contains("Defensive Security Architect")
            {
                "STRIDE Threat Model & Hardening roadmap.".to_string()
            } else if request.system_prompt.contains("Surface Mapper") {
                "Mapped entrypoints in scope.".to_string()
            } else if request
                .system_prompt
                .contains("Application Security Auditor")
            {
                "Found 1 HIGH path validation flaw.".to_string()
            } else if request.system_prompt.contains("Remediation & Patch") {
                "Drafted defensive path canonicalization patch.".to_string()
            } else if request.system_prompt.contains("STRIDE") {
                "STRIDE matrix complete.".to_string()
            } else {
                "Specialist analysis complete.".to_string()
            };

            if let Some(tx) = delta_tx {
                let _ = tx.send(content.clone());
            }

            Ok(CompletionResponse {
                content,
                tool_calls: Vec::new(),
                usage: Some(TokenUsage {
                    input_tokens: 10,
                    output_tokens: 5,
                }),
                stop_reason: Some("end_turn".into()),
                external_tools: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn security_defense_swarm_supports_folder_scope_and_synthesizes() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("src/auth")).unwrap();
        std::fs::write(tmp.path().join("src/auth/mod.rs"), "pub fn login() {}\n").unwrap();

        let engine = SwarmEngine::new(
            SwarmDomain::security_defense(),
            Arc::new(EchoRoleProvider),
            Arc::new(ToolRegistry::default_registry()),
            ToolContext::new(tmp.path().to_path_buf(), 10),
            SwarmOptions {
                target_scope: Some("src/auth".into()),
                ..SwarmOptions::default()
            },
        );

        let mut session = Session::new();
        let (tx, mut rx) = mpsc::unbounded_channel();

        engine
            .run_turn(&mut session, "Audit auth folder".into(), tx)
            .await;

        let mut badges = Vec::new();
        let mut saw_scope = None;
        let mut saw_synthesis = false;

        while let Ok(ev) = rx.try_recv() {
            match ev {
                SwarmEvent::SwarmStarted { scope, .. } => saw_scope = scope,
                SwarmEvent::WorkerReport { badge, .. } => badges.push(badge),
                SwarmEvent::SynthesisFinished {
                    lead_badge,
                    content,
                    ..
                } => {
                    assert_eq!(lead_badge, "DEFENSE LEAD");
                    assert!(content.contains("Defense Lead"));
                    saw_synthesis = true;
                }
                _ => {}
            }
        }

        badges.sort();
        assert_eq!(saw_scope.as_deref(), Some("src/auth"));
        assert_eq!(badges, vec!["AUDIT", "PATCH", "SURFACE"]);
        assert!(saw_synthesis);
    }

    #[tokio::test]
    async fn security_swarm_rejects_scope_escaping_workspace() {
        let tmp = TempDir::new().unwrap();
        let engine = SwarmEngine::new(
            SwarmDomain::security_defense(),
            Arc::new(EchoRoleProvider),
            Arc::new(ToolRegistry::default_registry()),
            ToolContext::new(tmp.path().to_path_buf(), 10),
            SwarmOptions {
                target_scope: Some("../../etc".into()),
                ..SwarmOptions::default()
            },
        );

        let mut session = Session::new();
        let (tx, mut rx) = mpsc::unbounded_channel();

        engine
            .run_turn(&mut session, "Audit outside".into(), tx)
            .await;

        let mut saw_error = false;
        while let Ok(ev) = rx.try_recv() {
            if let SwarmEvent::Error(msg) = ev {
                assert!(msg.contains("Invalid target scope"));
                saw_error = true;
            }
        }
        assert!(saw_error);
    }

    #[tokio::test]
    async fn security_threat_model_swarm_runs_stride_config_blueteam() {
        let tmp = TempDir::new().unwrap();
        let engine = SwarmEngine::new(
            SwarmDomain::security_threat(),
            Arc::new(EchoRoleProvider),
            Arc::new(ToolRegistry::default_registry()),
            ToolContext::new(tmp.path().to_path_buf(), 10),
            SwarmOptions::default(),
        );

        let mut session = Session::new();
        let (tx, mut rx) = mpsc::unbounded_channel();

        engine
            .run_turn(&mut session, "Model threats for this app".into(), tx)
            .await;

        let mut badges = Vec::new();
        let mut lead_ok = false;

        while let Ok(ev) = rx.try_recv() {
            match ev {
                SwarmEvent::WorkerReport { badge, .. } => badges.push(badge),
                SwarmEvent::SynthesisFinished { lead_badge, .. } => {
                    assert_eq!(lead_badge, "SEC ARCHITECT");
                    lead_ok = true;
                }
                _ => {}
            }
        }

        badges.sort();
        assert_eq!(badges, vec!["BLUETEAM", "CONFIG", "STRIDE"]);
        assert!(lead_ok);
    }
}
