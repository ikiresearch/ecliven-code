use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::agent::session::Session;
use crate::agent::{AgentEngine, AgentEvent, AgentOptions};
use crate::ai::registry::build_provider;
use crate::config::{AppConfig, ApprovalMode, ProviderKind};
use crate::swarm::domain::{SecuritySubMode, SwarmDomain};
use crate::swarm::{SwarmAgentStatus, SwarmEngine, SwarmEvent, SwarmOptions};
use crate::tools::{ToolContext, ToolOutput, ToolPreview, ToolRegistry};
use crate::tui::keybindings::ModalState;
use crate::updater::{self, ReleaseInfo, UpdateStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkspaceSection {
    Chat,
    SwarmCode,
    SwarmSecurity,
}

impl WorkspaceSection {
    pub fn next(&self) -> Self {
        match self {
            Self::Chat => Self::SwarmCode,
            Self::SwarmCode => Self::SwarmSecurity,
            Self::SwarmSecurity => Self::Chat,
        }
    }

    pub fn from_swarm_domain(domain: SwarmDomain) -> Self {
        match domain {
            SwarmDomain::Code => Self::SwarmCode,
            SwarmDomain::Security(_) => Self::SwarmSecurity,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::SwarmCode => "Swarm: Code",
            Self::SwarmSecurity => "Swarm: Security",
        }
    }
}

#[derive(Debug, Clone)]
pub enum ChatEntry {
    User(String),
    Assistant(String),
    SwarmReport {
        domain: SwarmDomain,
        badge: String,
        agent_name: String,
        content: String,
    },
    SwarmLead {
        domain: SwarmDomain,
        badge: String,
        content: String,
    },
    ToolCall {
        name: String,
        summary: String,
    },
    ToolResult {
        name: String,
        output: String,
        diff: Option<String>,
        is_error: bool,
    },
    System(String),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct SwarmWorkerView {
    pub id: String,
    pub name: String,
    pub badge: String,
    pub summary: String,
    pub status: SwarmAgentStatus,
}

pub struct SectionState {
    pub session: Arc<Mutex<Session>>,
    pub entries: Vec<ChatEntry>,
    pub streaming_buffer: String,
    pub scroll_offset: u16,
    pub auto_scroll: bool,
    pub roster: Vec<SwarmWorkerView>,
    pub lead_status: SwarmAgentStatus,
    pub target_scope: Option<String>,
}

impl SectionState {
    fn new_chat(welcome: String) -> Self {
        Self {
            session: Arc::new(Mutex::new(Session::new())),
            entries: vec![ChatEntry::System(welcome)],
            streaming_buffer: String::new(),
            scroll_offset: 0,
            auto_scroll: true,
            roster: Vec::new(),
            lead_status: SwarmAgentStatus::Idle,
            target_scope: None,
        }
    }

    fn new_swarm(domain: SwarmDomain, target_scope: Option<String>) -> Self {
        let roster = build_roster(domain);
        let specialist_names: Vec<&str> = domain.specialists().iter().map(|r| r.name).collect();
        let scope_note = target_scope
            .as_deref()
            .map(|s| format!("\nActive scope: `{s}` (change with /scope <folder> or /scope off)."))
            .unwrap_or_else(|| "\nScope: full workspace (focus a folder with /scope <folder>).".into());

        let intro = format!(
            "{} — {}\nParallel team: {} -> {}.{scope_note}\nPress F2 to switch tabs, F3 for Swarm Hub, or F4 to toggle Security sub-modes.",
            domain.display_name(),
            domain.subtitle(),
            specialist_names.join(" + "),
            domain.lead_badge()
        );

        Self {
            session: Arc::new(Mutex::new(Session::new())),
            entries: vec![ChatEntry::System(intro)],
            streaming_buffer: String::new(),
            scroll_offset: 0,
            auto_scroll: true,
            roster,
            lead_status: SwarmAgentStatus::Idle,
            target_scope,
        }
    }

    pub fn set_roster_for_domain(&mut self, domain: SwarmDomain) {
        self.roster = build_roster(domain);
        self.lead_status = SwarmAgentStatus::Idle;
    }

    pub fn reset_roster_idle(&mut self) {
        for worker in &mut self.roster {
            worker.status = SwarmAgentStatus::Idle;
        }
        self.lead_status = SwarmAgentStatus::Idle;
    }
}

fn build_roster(domain: SwarmDomain) -> Vec<SwarmWorkerView> {
    domain
        .specialists()
        .iter()
        .map(|r| SwarmWorkerView {
            id: r.id.to_string(),
            name: r.name.to_string(),
            badge: r.badge.to_string(),
            summary: r.summary.to_string(),
            status: SwarmAgentStatus::Idle,
        })
        .collect()
}

pub enum UiEvent {
    Agent {
        section: WorkspaceSection,
        event: AgentEvent,
    },
    Swarm {
        section: WorkspaceSection,
        domain: SwarmDomain,
        event: SwarmEvent,
    },
    UpdateAvailable(ReleaseInfo),
    UpdateCheckMessage(String),
    UpdateFinished(Result<String, String>),
}

pub struct PendingApproval {
    pub tool_name: String,
    pub preview: ToolPreview,
    pub respond_to: oneshot::Sender<bool>,
}

pub struct App {
    pub config: AppConfig,
    pub active_section: WorkspaceSection,
    pub security_sub_mode: SecuritySubMode,
    pub chat_state: SectionState,
    pub swarm_code_state: SectionState,
    pub swarm_security_state: SectionState,
    pub input: String,
    pub cursor_pos: usize,
    pub input_history: Vec<String>,
    pub history_index: Option<usize>,
    pub is_busy: bool,
    pub show_help: bool,
    pub show_swarm_hub: bool,
    pub should_quit: bool,
    pub tick_count: usize,
    pub total_input_tokens: u32,
    pub total_output_tokens: u32,
    pub available_update: Option<ReleaseInfo>,
    pub pending_approval: Option<PendingApproval>,
    tools: Arc<ToolRegistry>,
}

impl App {
    pub fn new(
        config: AppConfig,
        initial_swarm: Option<SwarmDomain>,
        initial_scope: Option<String>,
    ) -> Self {
        let provider_status = if config.active_provider.requires_api_key()
            && config.active_settings().api_key.is_none()
        {
            format!(
                "No API key detected for '{}'. Set {} or switch with /provider ollama.",
                config.active_provider,
                config
                    .active_provider
                    .env_key_name()
                    .unwrap_or("API_KEY")
            )
        } else {
            format!(
                "Ready with {}/{} in {}",
                config.active_provider,
                config.active_settings().model,
                config.workspace_dir.display()
            )
        };

        let welcome = format!(
            "Welcome to Ecliven Code v{}. {provider_status}\nPress F2 to switch tabs (Chat | Swarm: Code | Swarm: Security), F3 for Swarm Hub, or type /help.",
            env!("CARGO_PKG_VERSION")
        );

        let security_sub_mode = initial_swarm
            .and_then(|d| d.security_sub_mode())
            .unwrap_or(SecuritySubMode::Defense);

        let active_section = match initial_swarm {
            Some(domain) => WorkspaceSection::from_swarm_domain(domain),
            None => WorkspaceSection::Chat,
        };

        let code_scope = if active_section == WorkspaceSection::SwarmCode {
            initial_scope.clone()
        } else {
            None
        };
        let sec_scope = if active_section == WorkspaceSection::SwarmSecurity {
            initial_scope
        } else {
            None
        };

        Self {
            config,
            active_section,
            security_sub_mode,
            chat_state: SectionState::new_chat(welcome),
            swarm_code_state: SectionState::new_swarm(SwarmDomain::Code, code_scope),
            swarm_security_state: SectionState::new_swarm(
                SwarmDomain::Security(security_sub_mode),
                sec_scope,
            ),
            input: String::new(),
            cursor_pos: 0,
            input_history: Vec::new(),
            history_index: None,
            is_busy: false,
            show_help: false,
            show_swarm_hub: false,
            should_quit: false,
            tick_count: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            available_update: None,
            pending_approval: None,
            tools: Arc::new(ToolRegistry::default_registry()),
        }
    }

    pub fn active_swarm_domain(&self) -> Option<SwarmDomain> {
        match self.active_section {
            WorkspaceSection::Chat => None,
            WorkspaceSection::SwarmCode => Some(SwarmDomain::Code),
            WorkspaceSection::SwarmSecurity => Some(SwarmDomain::Security(self.security_sub_mode)),
        }
    }

    pub fn modal_state(&self) -> ModalState {
        if self.pending_approval.is_some() {
            ModalState::Approval
        } else if self.show_swarm_hub {
            ModalState::SwarmHub
        } else if self.show_help {
            ModalState::Help
        } else {
            ModalState::None
        }
    }

    pub fn active_state(&self) -> &SectionState {
        self.section_state(self.active_section)
    }

    pub fn active_state_mut(&mut self) -> &mut SectionState {
        self.section_state_mut(self.active_section)
    }

    pub fn section_state(&self, section: WorkspaceSection) -> &SectionState {
        match section {
            WorkspaceSection::Chat => &self.chat_state,
            WorkspaceSection::SwarmCode => &self.swarm_code_state,
            WorkspaceSection::SwarmSecurity => &self.swarm_security_state,
        }
    }

    pub fn section_state_mut(&mut self, section: WorkspaceSection) -> &mut SectionState {
        match section {
            WorkspaceSection::Chat => &mut self.chat_state,
            WorkspaceSection::SwarmCode => &mut self.swarm_code_state,
            WorkspaceSection::SwarmSecurity => &mut self.swarm_security_state,
        }
    }

    pub fn switch_section(&mut self, target: WorkspaceSection) {
        if self.is_busy {
            return;
        }
        self.active_section = target;
        self.show_swarm_hub = false;
        self.show_help = false;
    }

    pub fn set_security_sub_mode(&mut self, mode: SecuritySubMode) {
        if self.is_busy {
            return;
        }
        self.security_sub_mode = mode;
        let domain = SwarmDomain::Security(mode);
        self.swarm_security_state.set_roster_for_domain(domain);
        self.swarm_security_state.entries.push(ChatEntry::System(format!(
            "Switched Security Swarm mode to: {} — {}",
            mode.display_name(),
            domain.subtitle()
        )));
        self.swarm_security_state.auto_scroll = true;
        self.active_section = WorkspaceSection::SwarmSecurity;
        self.show_swarm_hub = false;
        self.show_help = false;
    }

    pub fn toggle_security_sub_mode(&mut self) {
        if self.is_busy {
            return;
        }
        let next = self.security_sub_mode.toggle();
        self.set_security_sub_mode(next);
    }

    pub fn cycle_section(&mut self) {
        if self.is_busy {
            return;
        }
        self.active_section = self.active_section.next();
    }

    pub fn close_modals(&mut self) {
        self.show_swarm_hub = false;
        self.show_help = false;
    }

    pub fn set_scope(&mut self, raw: Option<&str>) {
        let Some(path_str) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            self.active_state_mut().target_scope = None;
            self.active_state_mut()
                .entries
                .push(ChatEntry::System("Target scope cleared (full workspace).".into()));
            return;
        };

        if matches!(path_str.to_ascii_lowercase().as_str(), "off" | "none" | "all" | ".") {
            self.active_state_mut().target_scope = None;
            self.active_state_mut()
                .entries
                .push(ChatEntry::System("Target scope reset to full workspace.".into()));
            return;
        }

        let ctx = ToolContext::new(
            self.config.workspace_dir.clone(),
            self.config.shell_timeout_secs,
        );
        match ctx.resolve_path(path_str) {
            Ok(resolved) if resolved.exists() => {
                self.active_state_mut().target_scope = Some(path_str.to_string());
                self.active_state_mut().entries.push(ChatEntry::System(format!(
                    "Target scope set to `{path_str}`."
                )));
            }
            Ok(_) => {
                self.active_state_mut().entries.push(ChatEntry::Error(format!(
                    "Path '{path_str}' does not exist in workspace."
                )));
            }
            Err(err) => {
                self.active_state_mut()
                    .entries
                    .push(ChatEntry::Error(err.to_string()));
            }
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.input.insert(self.cursor_pos, c);
        self.cursor_pos += c.len_utf8();
    }

    pub fn backspace(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        let prev = self.input[..self.cursor_pos]
            .char_indices()
            .last()
            .map(|(idx, _)| idx)
            .unwrap_or(0);
        self.input.drain(prev..self.cursor_pos);
        self.cursor_pos = prev;
    }

    pub fn delete_char(&mut self) {
        if self.cursor_pos >= self.input.len() {
            return;
        }
        let next = self.input[self.cursor_pos..]
            .chars()
            .next()
            .map(|c| self.cursor_pos + c.len_utf8())
            .unwrap_or(self.cursor_pos);
        self.input.drain(self.cursor_pos..next);
    }

    pub fn move_left(&mut self) {
        if self.cursor_pos == 0 {
            return;
        }
        self.cursor_pos = self.input[..self.cursor_pos]
            .char_indices()
            .last()
            .map(|(idx, _)| idx)
            .unwrap_or(0);
    }

    pub fn move_right(&mut self) {
        if self.cursor_pos >= self.input.len() {
            return;
        }
        if let Some(c) = self.input[self.cursor_pos..].chars().next() {
            self.cursor_pos += c.len_utf8();
        }
    }

    pub fn move_home(&mut self) {
        self.cursor_pos = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor_pos = self.input.len();
    }

    pub fn clear_input(&mut self) {
        if self.input.is_empty() && !self.is_busy {
            self.should_quit = true;
            return;
        }
        self.input.clear();
        self.cursor_pos = 0;
        self.history_index = None;
    }

    pub fn history_prev(&mut self) {
        if self.input_history.is_empty() {
            return;
        }
        let next_idx = match self.history_index {
            None => self.input_history.len() - 1,
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.history_index = Some(next_idx);
        self.input = self.input_history[next_idx].clone();
        self.cursor_pos = self.input.len();
    }

    pub fn history_next(&mut self) {
        let Some(curr) = self.history_index else {
            return;
        };
        if curr + 1 >= self.input_history.len() {
            self.history_index = None;
            self.input.clear();
            self.cursor_pos = 0;
        } else {
            let next = curr + 1;
            self.history_index = Some(next);
            self.input = self.input_history[next].clone();
            self.cursor_pos = self.input.len();
        }
    }

    pub fn scroll_up(&mut self, amount: u16) {
        let st = self.active_state_mut();
        st.auto_scroll = false;
        st.scroll_offset = st.scroll_offset.saturating_sub(amount);
    }

    pub fn scroll_down(&mut self, amount: u16) {
        let st = self.active_state_mut();
        st.scroll_offset = st.scroll_offset.saturating_add(amount);
    }

    pub fn resolve_approval(&mut self, approved: bool) {
        if let Some(pending) = self.pending_approval.take() {
            let _ = pending.respond_to.send(approved);
        }
    }

    pub async fn clear_session(&mut self) {
        if self.is_busy {
            return;
        }
        let label = self.active_section.label();
        let st = self.active_state_mut();
        st.session.lock().await.clear();
        st.entries.clear();
        st.streaming_buffer.clear();
        st.reset_roster_idle();
        st.entries
            .push(ChatEntry::System(format!("{label} session cleared.")));
        st.auto_scroll = true;
    }

    pub async fn submit_input(&mut self, ui_tx: mpsc::UnboundedSender<UiEvent>) {
        if self.is_busy {
            return;
        }
        let prompt = self.input.trim().to_string();
        if prompt.is_empty() {
            return;
        }

        self.input_history.push(prompt.clone());
        self.history_index = None;
        self.input.clear();
        self.cursor_pos = 0;
        self.active_state_mut().auto_scroll = true;

        if prompt.starts_with('/') {
            self.handle_slash_command(&prompt, ui_tx).await;
            return;
        }

        let section = self.active_section;
        self.section_state_mut(section)
            .entries
            .push(ChatEntry::User(prompt.clone()));

        let provider = match build_provider(&self.config) {
            Ok(p) => p,
            Err(err) => {
                self.section_state_mut(section)
                    .entries
                    .push(ChatEntry::Error(err.to_string()));
                return;
            }
        };

        let tool_ctx = ToolContext::new(
            self.config.workspace_dir.clone(),
            self.config.shell_timeout_secs,
        );
        let session = Arc::clone(&self.section_state(section).session);
        let target_scope = self.section_state(section).target_scope.clone();
        self.is_busy = true;
        self.section_state_mut(section).streaming_buffer.clear();

        if let Some(domain) = self.active_swarm_domain() {
            self.section_state_mut(section).reset_roster_idle();

            let engine = SwarmEngine::new(
                domain,
                provider,
                Arc::clone(&self.tools),
                tool_ctx,
                SwarmOptions {
                    max_tokens: self.config.max_tokens,
                    temperature: self.config.temperature,
                    worker_max_iterations: 8,
                    lead_max_iterations: self.config.max_tool_iterations,
                    approval_mode: self.config.approval_mode,
                    target_scope,
                },
            );

            tokio::spawn(async move {
                let (tx, mut rx) = mpsc::unbounded_channel::<SwarmEvent>();
                let fwd = tokio::spawn(async move {
                    while let Some(event) = rx.recv().await {
                        let _ = ui_tx.send(UiEvent::Swarm {
                            section,
                            domain,
                            event,
                        });
                    }
                });
                let mut guard = session.lock().await;
                engine.run_turn(&mut guard, prompt, tx).await;
                let _ = fwd.await;
            });
        } else {
            let engine = AgentEngine::new(
                provider,
                Arc::clone(&self.tools),
                tool_ctx,
                AgentOptions {
                    max_tokens: self.config.max_tokens,
                    temperature: self.config.temperature,
                    max_tool_iterations: self.config.max_tool_iterations,
                    approval_mode: self.config.approval_mode,
                },
            );

            tokio::spawn(async move {
                let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();
                let fwd = tokio::spawn(async move {
                    while let Some(event) = rx.recv().await {
                        let _ = ui_tx.send(UiEvent::Agent { section, event });
                    }
                });
                let mut guard = session.lock().await;
                engine.run_turn(&mut guard, prompt, tx).await;
                let _ = fwd.await;
            });
        }
    }

    pub fn handle_ui_event(&mut self, event: UiEvent) {
        match event {
            UiEvent::Agent { section, event } => self.handle_agent_event(section, event),
            UiEvent::Swarm {
                section,
                domain,
                event,
            } => self.handle_swarm_event(section, domain, event),
            UiEvent::UpdateAvailable(info) => {
                let msg = format!(
                    "Update available: v{} -> v{} ({}). Run /update or 'ecliven update' to upgrade.",
                    updater::CURRENT_VERSION,
                    info.version,
                    info.html_url
                );
                self.available_update = Some(info);
                self.active_state_mut().entries.push(ChatEntry::System(msg));
                self.active_state_mut().auto_scroll = true;
            }
            UiEvent::UpdateCheckMessage(msg) => {
                self.active_state_mut().entries.push(ChatEntry::System(msg));
                self.active_state_mut().auto_scroll = true;
            }
            UiEvent::UpdateFinished(result) => {
                self.is_busy = false;
                match result {
                    Ok(out) => {
                        self.available_update = None;
                        self.active_state_mut().entries.push(ChatEntry::System(format!(
                            "Update completed!\n{out}\nRestart 'ecliven' to run the new version."
                        )));
                    }
                    Err(err) => {
                        self.active_state_mut()
                            .entries
                            .push(ChatEntry::Error(format!("Update failed: {err}")));
                    }
                }
                self.active_state_mut().auto_scroll = true;
            }
        }
    }

    fn handle_agent_event(&mut self, section: WorkspaceSection, event: AgentEvent) {
        match event {
            AgentEvent::AssistantDelta(chunk) => {
                let st = self.section_state_mut(section);
                st.streaming_buffer.push_str(&chunk);
                st.auto_scroll = true;
            }
            AgentEvent::AssistantFinished { content, usage } => {
                let st = self.section_state_mut(section);
                st.streaming_buffer.clear();
                if !content.trim().is_empty() {
                    st.entries.push(ChatEntry::Assistant(content));
                }
                st.auto_scroll = true;
                if let Some(u) = usage {
                    self.total_input_tokens =
                        self.total_input_tokens.saturating_add(u.input_tokens);
                    self.total_output_tokens =
                        self.total_output_tokens.saturating_add(u.output_tokens);
                }
            }
            AgentEvent::ToolApprovalRequested {
                tool_name,
                preview,
                respond_to,
                ..
            } => {
                self.pending_approval = Some(PendingApproval {
                    tool_name,
                    preview,
                    respond_to,
                });
            }
            AgentEvent::ToolStarted { tool_name, summary } => {
                let st = self.section_state_mut(section);
                st.entries.push(ChatEntry::ToolCall {
                    name: tool_name,
                    summary,
                });
                st.auto_scroll = true;
            }
            AgentEvent::ToolCompleted {
                tool_name,
                output:
                    ToolOutput {
                        content,
                        diff,
                        is_error,
                    },
            } => {
                let preview_text = truncate_lines(&content, 24);
                let st = self.section_state_mut(section);
                st.entries.push(ChatEntry::ToolResult {
                    name: tool_name,
                    output: preview_text,
                    diff,
                    is_error,
                });
                st.auto_scroll = true;
            }
            AgentEvent::Error(err) => {
                let st = self.section_state_mut(section);
                st.streaming_buffer.clear();
                st.entries.push(ChatEntry::Error(err));
                st.auto_scroll = true;
            }
            AgentEvent::TurnComplete => {
                self.is_busy = false;
                let st = self.section_state_mut(section);
                st.streaming_buffer.clear();
                st.auto_scroll = true;
            }
        }
    }

    fn handle_swarm_event(
        &mut self,
        section: WorkspaceSection,
        domain: SwarmDomain,
        event: SwarmEvent,
    ) {
        match event {
            SwarmEvent::SwarmStarted { scope, .. } => {
                let st = self.section_state_mut(section);
                st.reset_roster_idle();
                let scope_info = scope
                    .map(|s| format!(" on scope `{s}`"))
                    .unwrap_or_default();
                st.entries.push(ChatEntry::System(format!(
                    "Launching {} specialists in parallel{scope_info}...",
                    domain.display_name()
                )));
                st.auto_scroll = true;
            }
            SwarmEvent::WorkerStatus {
                agent_id, status, ..
            } => {
                let st = self.section_state_mut(section);
                if let Some(worker) = st.roster.iter_mut().find(|w| w.id == agent_id) {
                    worker.status = status;
                }
            }
            SwarmEvent::WorkerReport {
                agent_name,
                badge,
                content,
                usage,
                ..
            } => {
                if !content.trim().is_empty() {
                    let st = self.section_state_mut(section);
                    st.entries.push(ChatEntry::SwarmReport {
                        domain,
                        badge,
                        agent_name,
                        content: truncate_lines(&content, 30),
                    });
                    st.auto_scroll = true;
                }
                if let Some(u) = usage {
                    self.total_input_tokens =
                        self.total_input_tokens.saturating_add(u.input_tokens);
                    self.total_output_tokens =
                        self.total_output_tokens.saturating_add(u.output_tokens);
                }
            }
            SwarmEvent::Synthesizing { .. } => {
                let st = self.section_state_mut(section);
                st.lead_status = SwarmAgentStatus::Running { active_tool: None };
                st.streaming_buffer.clear();
            }
            SwarmEvent::SynthesisDelta(chunk) => {
                let st = self.section_state_mut(section);
                st.streaming_buffer.push_str(&chunk);
                st.auto_scroll = true;
            }
            SwarmEvent::SynthesisFinished {
                lead_badge,
                content,
                usage,
            } => {
                let st = self.section_state_mut(section);
                st.streaming_buffer.clear();
                st.lead_status = SwarmAgentStatus::Done;
                if !content.trim().is_empty() {
                    st.entries.push(ChatEntry::SwarmLead {
                        domain,
                        badge: lead_badge,
                        content,
                    });
                }
                st.auto_scroll = true;
                if let Some(u) = usage {
                    self.total_input_tokens =
                        self.total_input_tokens.saturating_add(u.input_tokens);
                    self.total_output_tokens =
                        self.total_output_tokens.saturating_add(u.output_tokens);
                }
            }
            SwarmEvent::ToolApprovalRequested {
                tool_name,
                preview,
                respond_to,
                ..
            } => {
                self.pending_approval = Some(PendingApproval {
                    tool_name,
                    preview,
                    respond_to,
                });
            }
            SwarmEvent::ToolStarted { tool_name, summary } => {
                let st = self.section_state_mut(section);
                st.lead_status = SwarmAgentStatus::Running {
                    active_tool: Some(tool_name.clone()),
                };
                st.entries.push(ChatEntry::ToolCall {
                    name: tool_name,
                    summary,
                });
                st.auto_scroll = true;
            }
            SwarmEvent::ToolCompleted {
                tool_name,
                output:
                    ToolOutput {
                        content,
                        diff,
                        is_error,
                    },
            } => {
                let preview_text = truncate_lines(&content, 24);
                let st = self.section_state_mut(section);
                st.entries.push(ChatEntry::ToolResult {
                    name: tool_name,
                    output: preview_text,
                    diff,
                    is_error,
                });
                st.auto_scroll = true;
            }
            SwarmEvent::Error(err) => {
                let st = self.section_state_mut(section);
                st.streaming_buffer.clear();
                st.lead_status = SwarmAgentStatus::Failed(err.clone());
                st.entries.push(ChatEntry::Error(err));
                st.auto_scroll = true;
            }
            SwarmEvent::SwarmComplete => {
                self.is_busy = false;
                let st = self.section_state_mut(section);
                st.streaming_buffer.clear();
                st.auto_scroll = true;
            }
        }
    }

    async fn handle_slash_command(
        &mut self,
        raw: &str,
        ui_tx: mpsc::UnboundedSender<UiEvent>,
    ) {
        let mut parts = raw.split_whitespace();
        let cmd = parts.next().unwrap_or("");
        let arg1 = parts.next();
        let arg2 = parts.next();
        let arg3 = parts.next();

        match cmd {
            "/help" => {
                self.active_state_mut().entries.push(ChatEntry::System(
                    "Slash commands:\n\
                     /swarm                                          Open Swarm Hub (Code vs Security)\n\
                     /swarm code [folder]                            Switch to Swarm: Code (3 coders -> Chief)\n\
                     /swarm security defense [folder]                Security Defense: folder/code vulnerability audit\n\
                     /swarm security threat [folder]                 Security Threat Model (STRIDE / Config / Deps)\n\
                     /swarm off  (or /chat)                          Return to standard single-agent Chat\n\
                     /scope <folder|off>                             Focus active Swarm on a specific folder/file\n\
                     /update [check]                                 Check for updates or self-update Ecliven CLI\n\
                     /provider <anthropic|openai|openrouter|ollama>  Switch active AI provider\n\
                     /model <model-name>                             Switch active model\n\
                     /approval <ask|auto-edit|auto-all>              Set tool approval policy\n\
                     /status                                         Show session & provider status\n\
                     /clear                                          Clear current tab history\n\
                     /quit                                           Exit Ecliven Code\n\n\
                     Shortcuts: F2=Switch Tab | F3=Swarm Hub | F4=Toggle Security Mode | Enter=Send | F1=Help | Ctrl+Q=Quit"
                        .to_string(),
                ));
            }
            "/swarm" => match arg1 {
                None => {
                    self.show_swarm_hub = true;
                }
                Some("off" | "none" | "chat" | "normal") => {
                    self.switch_section(WorkspaceSection::Chat);
                }
                Some(domain_str) => match SwarmDomain::parse(domain_str) {
                    Some(SwarmDomain::Code) => {
                        self.switch_section(WorkspaceSection::SwarmCode);
                        if let Some(scope) = arg2 {
                            self.set_scope(Some(scope));
                        }
                    }
                    Some(SwarmDomain::Security(default_sub)) => {
                        let (sub, scope) = match arg2.and_then(SecuritySubMode::parse) {
                            Some(parsed_sub) => (parsed_sub, arg3),
                            None => (default_sub, arg2),
                        };
                        self.set_security_sub_mode(sub);
                        if let Some(sc) = scope {
                            self.set_scope(Some(sc));
                        }
                    }
                    None => {
                        self.active_state_mut().entries.push(ChatEntry::Error(format!(
                            "Unknown swarm domain '{domain_str}'. Use: /swarm code | /swarm security defense [folder] | /swarm security threat | /swarm off"
                        )));
                    }
                },
            },
            "/scope" => {
                self.set_scope(arg1);
            }
            "/update" => {
                let check_only = matches!(arg1, Some("check" | "--check"));
                if check_only {
                    self.active_state_mut()
                        .entries
                        .push(ChatEntry::System("Checking GitHub for updates...".into()));
                    tokio::spawn(async move {
                        let event = match updater::check_for_update().await {
                            Ok(UpdateStatus::Available { latest, .. }) => {
                                UiEvent::UpdateAvailable(latest)
                            }
                            Ok(UpdateStatus::UpToDate { current }) => UiEvent::UpdateCheckMessage(
                                format!("Ecliven Code v{current} is already up to date."),
                            ),
                            Err(err) => UiEvent::UpdateCheckMessage(format!(
                                "Could not check for updates: {err}"
                            )),
                        };
                        let _ = ui_tx.send(event);
                    });
                } else {
                    self.is_busy = true;
                    let target = self
                        .available_update
                        .as_ref()
                        .map(|u| u.tag_name.clone());
                    self.active_state_mut().entries.push(ChatEntry::System(
                        "Downloading and installing latest Ecliven Code release...".into(),
                    ));
                    tokio::spawn(async move {
                        let res = updater::run_self_update(target.as_deref())
                            .await
                            .map_err(|e| e.to_string());
                        let _ = ui_tx.send(UiEvent::UpdateFinished(res));
                    });
                }
            }
            "/chat" => {
                self.switch_section(WorkspaceSection::Chat);
            }
            "/provider" => match arg1 {
                Some(name) => match ProviderKind::parse(name) {
                    Ok(kind) => {
                        self.config.set_provider(kind);
                        let msg = format!(
                            "Switched provider to {} (model: {}).",
                            kind,
                            self.config.active_settings().model
                        );
                        self.active_state_mut().entries.push(ChatEntry::System(msg));
                    }
                    Err(e) => self
                        .active_state_mut()
                        .entries
                        .push(ChatEntry::Error(e.to_string())),
                },
                None => {
                    let msg = format!(
                        "Current provider: {}. Usage: /provider <anthropic|openai|openrouter|ollama>",
                        self.config.active_provider
                    );
                    self.active_state_mut().entries.push(ChatEntry::System(msg));
                }
            },
            "/model" => match arg1 {
                Some(model) => {
                    self.config.set_model(model);
                    let msg = format!(
                        "Active model for {} set to '{}'.",
                        self.config.active_provider, model
                    );
                    self.active_state_mut().entries.push(ChatEntry::System(msg));
                }
                None => {
                    let msg = format!(
                        "Current model: {}. Usage: /model <model-name>",
                        self.config.active_settings().model
                    );
                    self.active_state_mut().entries.push(ChatEntry::System(msg));
                }
            },
            "/approval" => match arg1 {
                Some("ask") => {
                    self.config.approval_mode = ApprovalMode::Ask;
                    self.active_state_mut()
                        .entries
                        .push(ChatEntry::System("Approval mode set to 'ask'.".into()));
                }
                Some("auto-edit") => {
                    self.config.approval_mode = ApprovalMode::AutoEdit;
                    self.active_state_mut()
                        .entries
                        .push(ChatEntry::System("Approval mode set to 'auto-edit'.".into()));
                }
                Some("auto-all") => {
                    self.config.approval_mode = ApprovalMode::AutoAll;
                    self.active_state_mut()
                        .entries
                        .push(ChatEntry::System("Approval mode set to 'auto-all'.".into()));
                }
                _ => {
                    let msg = format!(
                        "Current approval mode: {:?}. Usage: /approval <ask|auto-edit|auto-all>",
                        self.config.approval_mode
                    );
                    self.active_state_mut().entries.push(ChatEntry::System(msg));
                }
            },
            "/status" => {
                let settings = self.config.active_settings();
                let key_status = if !self.config.active_provider.requires_api_key() {
                    "not required (local)"
                } else if settings.api_key.is_some() {
                    "configured"
                } else {
                    "MISSING"
                };
                let section_desc = match self.active_swarm_domain() {
                    Some(d) => d.display_name(),
                    None => self.active_section.label(),
                };
                let scope_desc = self
                    .active_state()
                    .target_scope
                    .as_deref()
                    .unwrap_or("(full workspace)");
                let msg = format!(
                    "Section:   {section_desc}\nScope:     {scope_desc}\nWorkspace: {}\nProvider:  {} ({})\nModel:     {}\nEndpoint:  {}\nApproval:  {:?}\nTokens:    in={} out={}",
                    self.config.workspace_dir.display(),
                    self.config.active_provider,
                    key_status,
                    settings.model,
                    settings.base_url,
                    self.config.approval_mode,
                    self.total_input_tokens,
                    self.total_output_tokens
                );
                self.active_state_mut().entries.push(ChatEntry::System(msg));
            }
            "/clear" => {
                self.clear_session().await;
            }
            "/quit" | "/exit" => {
                self.should_quit = true;
            }
            other => {
                self.active_state_mut().entries.push(ChatEntry::Error(format!(
                    "Unknown command '{other}'. Type /help for available commands."
                )));
            }
        }
    }
}

fn truncate_lines(text: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max_lines {
        return text.to_string();
    }
    let mut out = lines[..max_lines].join("\n");
    out.push_str(&format!("\n... ({} more lines)", lines.len() - max_lines));
    out
}
