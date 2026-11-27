use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::agent::compact::compact_session;
use crate::agent::session::Session;
use crate::agent::store::{self, SessionStore};
use crate::agent::system_prompt::build_system_prompt;
use crate::agent::{AgentEngine, AgentEvent, AgentOptions};
use crate::ai::cli_backend::{detect_installed_clis, DetectedCli};
use crate::ai::models::{curated_fallback_models, fetch_models_for_config, ModelOption};
use crate::ai::registry::build_provider;
use crate::ai::Role;
use crate::config::preferences::UserPreferences;
use crate::config::{AppConfig, ApprovalMode, ProviderKind};
use crate::mcp::McpConnections;
use crate::swarm::domain::{SecuritySubMode, SwarmDomain};
use crate::swarm::{SwarmAgentStatus, SwarmEngine, SwarmEvent, SwarmOptions};
use crate::tools::sandbox::SandboxMode;
use crate::tools::{ToolContext, ToolOutput, ToolPreview, ToolRegistry};
use crate::tui::keybindings::ModalState;
use crate::tui::onboarding::{OnboardingState, OnboardingStep};
use crate::updater::{self, ReleaseInfo, UpdateStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkspaceSection {
    Chat,
    SwarmCode,
    SwarmSecurity,
    /// Competitive, collaborative or Red Eclipse swarm (see `App::team_domain`).
    SwarmTeam,
}

impl WorkspaceSection {
    pub fn next(&self) -> Self {
        match self {
            Self::Chat => Self::SwarmCode,
            Self::SwarmCode => Self::SwarmSecurity,
            Self::SwarmSecurity => Self::SwarmTeam,
            Self::SwarmTeam => Self::Chat,
        }
    }

    pub fn from_swarm_domain(domain: SwarmDomain) -> Self {
        match domain {
            SwarmDomain::Code => Self::SwarmCode,
            SwarmDomain::Security(_) => Self::SwarmSecurity,
            SwarmDomain::Competitive | SwarmDomain::Collaborative | SwarmDomain::RedEclipse => {
                Self::SwarmTeam
            }
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::SwarmCode => "Swarm: Code",
            Self::SwarmSecurity => "Swarm: Security",
            Self::SwarmTeam => "Swarm: Team",
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
    /// Prompt tokens reported by the latest response in this tab.
    pub context_tokens: u32,
}

impl SectionState {
    fn new_chat() -> Self {
        Self {
            session: Arc::new(Mutex::new(Session::new())),
            entries: Vec::new(),
            streaming_buffer: String::new(),
            scroll_offset: 0,
            auto_scroll: true,
            roster: Vec::new(),
            lead_status: SwarmAgentStatus::Idle,
            target_scope: None,
            context_tokens: 0,
        }
    }

    fn new_swarm(domain: SwarmDomain, target_scope: Option<String>) -> Self {
        let roster = build_roster(domain);
        Self {
            session: Arc::new(Mutex::new(Session::new())),
            entries: Vec::new(),
            streaming_buffer: String::new(),
            scroll_offset: 0,
            auto_scroll: true,
            roster,
            lead_status: SwarmAgentStatus::Idle,
            target_scope,
            context_tokens: 0,
        }
    }

    pub fn is_landing(&self) -> bool {
        self.entries.is_empty() && self.streaming_buffer.is_empty()
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
    ModelsLoaded {
        provider: ProviderKind,
        models: Vec<ModelOption>,
    },
    McpConnected(McpConnections),
}

#[derive(Debug, Clone)]
pub struct ModelPickerState {
    pub provider: ProviderKind,
    pub models: Vec<ModelOption>,
    pub filter: String,
    pub selected: usize,
    pub loading: bool,
}

impl ModelPickerState {
    pub fn filtered_models(&self) -> Vec<&ModelOption> {
        let needle = self.filter.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return self.models.iter().collect();
        }
        self.models
            .iter()
            .filter(|m| {
                m.id.to_ascii_lowercase().contains(&needle)
                    || m.label.to_ascii_lowercase().contains(&needle)
            })
            .collect()
    }
}

pub struct PendingApproval {
    pub tool_name: String,
    pub preview: ToolPreview,
    pub respond_to: oneshot::Sender<bool>,
}

pub struct App {
    pub config: AppConfig,
    pub prefs: UserPreferences,
    prefs_path: Option<PathBuf>,
    pub onboarding: Option<OnboardingState>,
    pub active_section: WorkspaceSection,
    pub security_sub_mode: SecuritySubMode,
    pub team_domain: SwarmDomain,
    pub chat_state: SectionState,
    pub swarm_code_state: SectionState,
    pub swarm_security_state: SectionState,
    pub swarm_team_state: SectionState,
    pub input: String,
    pub cursor_pos: usize,
    pub input_history: Vec<String>,
    pub history_index: Option<usize>,
    pub is_busy: bool,
    pub show_help: bool,
    pub show_swarm_hub: bool,
    pub should_quit: bool,
    pub tick_count: usize,
    pub started_at: Instant,
    pub total_input_tokens: u32,
    pub total_cache_read_tokens: u32,
    pub total_output_tokens: u32,
    pub available_update: Option<ReleaseInfo>,
    pub detected_clis: Vec<DetectedCli>,
    pub model_cache: HashMap<ProviderKind, Vec<ModelOption>>,
    pub model_picker: Option<ModelPickerState>,
    pub pending_approval: Option<PendingApproval>,
    tools: Arc<ToolRegistry>,
    session_store: Option<SessionStore>,
    mcp: Option<McpConnections>,
    /// Context for the model (e.g. after /undo) prepended to the next prompt.
    pending_notes: Vec<String>,
}

impl App {
    pub fn new(
        config: AppConfig,
        initial_swarm: Option<SwarmDomain>,
        initial_scope: Option<String>,
    ) -> Self {
        let mut prefs = UserPreferences::default();
        prefs.accept_terms();
        Self::with_preferences(config, initial_swarm, initial_scope, prefs, None, false)
    }

    pub fn with_preferences(
        config: AppConfig,
        initial_swarm: Option<SwarmDomain>,
        initial_scope: Option<String>,
        prefs: UserPreferences,
        prefs_path: Option<PathBuf>,
        force_setup: bool,
    ) -> Self {
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
            initial_scope.clone()
        } else {
            None
        };
        let team_domain = initial_swarm
            .filter(SwarmDomain::is_team_mode)
            .unwrap_or(SwarmDomain::Competitive);
        let team_scope = if active_section == WorkspaceSection::SwarmTeam {
            initial_scope
        } else {
            None
        };

        let onboarding = if force_setup {
            Some(OnboardingState::new_full(
                config.workspace_dir.clone(),
                prefs.telemetry_enabled,
                !prefs.needs_onboarding(),
            ))
        } else if prefs.needs_onboarding() {
            Some(OnboardingState::new_full(
                config.workspace_dir.clone(),
                prefs.telemetry_enabled,
                false,
            ))
        } else {
            None
        };

        Self {
            config,
            prefs,
            prefs_path,
            onboarding,
            active_section,
            security_sub_mode,
            team_domain,
            chat_state: SectionState::new_chat(),
            swarm_code_state: SectionState::new_swarm(SwarmDomain::Code, code_scope),
            swarm_security_state: SectionState::new_swarm(
                SwarmDomain::Security(security_sub_mode),
                sec_scope,
            ),
            swarm_team_state: SectionState::new_swarm(team_domain, team_scope),
            input: String::new(),
            cursor_pos: 0,
            input_history: Vec::new(),
            history_index: None,
            is_busy: false,
            show_help: false,
            show_swarm_hub: false,
            should_quit: false,
            tick_count: 0,
            started_at: Instant::now(),
            total_input_tokens: 0,
            total_cache_read_tokens: 0,
            total_output_tokens: 0,
            available_update: None,
            detected_clis: detect_installed_clis(),
            model_cache: HashMap::new(),
            model_picker: None,
            pending_approval: None,
            tools: Arc::new(ToolRegistry::default_registry()),
            session_store: None,
            mcp: None,
            pending_notes: Vec::new(),
        }
    }

    /// Enables on-disk session recording (`/sessions`, `/resume`).
    pub fn set_session_store(&mut self, store: Option<SessionStore>) {
        self.session_store = store;
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tools.names()
    }

    /// Connects configured MCP servers in the background.
    pub fn start_mcp(&mut self, ui_tx: mpsc::UnboundedSender<UiEvent>) {
        let servers = self.config.mcp_servers.clone();
        if servers.iter().all(|s| !s.enabled) {
            return;
        }
        tokio::spawn(async move {
            let conns = crate::mcp::connect_all(&servers).await;
            let _ = ui_tx.send(UiEvent::McpConnected(conns));
        });
    }

    pub fn announce_workspace_trust(&mut self) {
        if self.config.workspace_trust.withheld() {
            let msg = format!(
                "{}\nUse /trust to enable them or /trust deny to keep them off.",
                self.config.workspace_trust.describe()
            );
            self.push_system(msg);
        }
    }

    fn handle_trust_command(&mut self, arg: Option<&str>, ui_tx: mpsc::UnboundedSender<UiEvent>) {
        use crate::config::trust::{TrustStatus, TrustStore};
        let trust = self.config.workspace_trust.clone();
        let result = match arg {
            Some("status") => return self.push_system(trust.describe()),
            Some("revoke") => {
                let mut store = TrustStore::load_default();
                store.revoke(&trust.key);
                store.save().map(|_| "Trust decision removed; restart to be asked again.".to_string())
            }
            Some("deny") => TrustStore::decide(&trust, false).map(|_| {
                self.config.workspace_trust.status = TrustStatus::Denied;
                "Workspace denied: executable project settings stay disabled.".to_string()
            }),
            None | Some("yes") => {
                if self.is_busy {
                    return self.push_error("Wait for the current turn to finish.");
                }
                TrustStore::decide(&trust, true).and_then(|_| {
                    let mut cfg = self.config.reload().map_err(|e| e.to_string())?;
                    let model = self.config.active_settings().model.clone();
                    cfg.set_provider(self.config.active_provider);
                    cfg.set_model(model);
                    self.config = cfg;
                    Ok(format!(
                        "Workspace trusted. Approval: {:?}, sandbox: {:?}, MCP servers: {}.",
                        self.config.approval_mode,
                        self.config.sandbox_mode,
                        self.config.mcp_servers.len()
                    ))
                })
            }
            Some(_) => return self.push_error("Usage: /trust [yes|deny|revoke|status]"),
        };
        match result {
            Ok(msg) => {
                self.push_system(msg);
                if matches!(arg, None | Some("yes")) {
                    self.start_mcp(ui_tx);
                }
            }
            Err(e) => self.push_error(format!("Could not save trust decision: {e}")),
        }
    }

    fn push_system(&mut self, msg: impl Into<String>) {
        let st = self.active_state_mut();
        st.entries.push(ChatEntry::System(msg.into()));
        st.auto_scroll = true;
    }

    fn push_error(&mut self, msg: impl Into<String>) {
        let st = self.active_state_mut();
        st.entries.push(ChatEntry::Error(msg.into()));
        st.auto_scroll = true;
    }

    /// Loads a saved session into the active tab.
    pub async fn resume_session(&mut self, query: &str) {
        if self.is_busy {
            return;
        }
        let Some(store) = self.session_store.clone() else {
            self.push_error("Session storage is not available.");
            return;
        };
        let found = match store.find(query, &self.config.workspace_dir) {
            Ok(f) => f,
            Err(err) => return self.push_error(err),
        };
        let loaded = match store::load_session(&found.path) {
            Ok(l) => l,
            Err(err) => return self.push_error(err),
        };
        let messages = loaded.messages.clone();
        let mut entries = Vec::new();
        for m in &messages {
            match m.role {
                Role::User => entries.push(ChatEntry::User(m.content.clone())),
                Role::Assistant if !m.content.trim().is_empty() => {
                    entries.push(ChatEntry::Assistant(m.content.clone()))
                }
                _ => {}
            }
        }
        let count = messages.len();
        let mut session = Session::from_loaded(loaded);
        session.attach_recorder(store.reopen(&found));
        let st = self.active_state_mut();
        *st.session.lock().await = session;
        st.entries = entries;
        st.streaming_buffer.clear();
        st.scroll_offset = 0;
        self.push_system(format!(
            "Resumed session {} ({count} messages): {}",
            found.meta.id, found.meta.title
        ));
    }

    fn list_sessions(&mut self) {
        let Some(store) = &self.session_store else {
            return self.push_error("Session storage is not available.");
        };
        let sessions = store.list(Some(&self.config.workspace_dir));
        if sessions.is_empty() {
            return self.push_system("No saved sessions for this workspace.");
        }
        let lines: Vec<String> = sessions
            .iter()
            .take(20)
            .map(|s| {
                format!(
                    "{}  {:>9}  {:>3} msgs  {}",
                    s.meta.id,
                    store::format_age(s.updated_at),
                    s.message_count,
                    s.meta.title
                )
            })
            .collect();
        self.push_system(format!(
            "Saved sessions (newest first). Use /resume <id>:\n{}",
            lines.join("\n")
        ));
    }

    async fn undo_last_checkpoint(&mut self) {
        if self.is_busy {
            return;
        }
        let session = Arc::clone(&self.active_state().session);
        let checkpoint = session.lock().await.pop_checkpoint();
        let Some(cp) = checkpoint else {
            return self.push_system("Nothing to undo: no file changes recorded in this tab.");
        };
        match cp.restore() {
            Ok(report) => {
                let root = self.config.workspace_dir.clone();
                let names: Vec<String> = report
                    .restored
                    .iter()
                    .map(|p| p.strip_prefix(&root).unwrap_or(p).display().to_string())
                    .collect();
                if !report.skipped.is_empty() {
                    let skipped: Vec<String> = report
                        .skipped
                        .iter()
                        .map(|(p, why)| {
                            format!("{} ({why})", p.strip_prefix(&root).unwrap_or(p).display())
                        })
                        .collect();
                    self.push_error(format!(
                        "Could not restore (no snapshot, left untouched): {}",
                        skipped.join(", ")
                    ));
                }
                self.pending_notes.push(format!(
                    "The user undid your file changes from the turn \"{}\"; these files were restored: {}.",
                    cp.label,
                    names.join(", ")
                ));
                self.push_system(format!(
                    "Undid changes from \"{}\". Restored: {}",
                    cp.label,
                    names.join(", ")
                ));
            }
            Err(err) => self.push_error(format!("Undo failed: {err}")),
        }
    }

    async fn list_checkpoints(&mut self) {
        let session = Arc::clone(&self.active_state().session);
        let guard = session.lock().await;
        let root = self.config.workspace_dir.clone();
        let lines: Vec<String> = guard
            .checkpoints()
            .map(|cp| {
                let files: Vec<String> = cp
                    .paths()
                    .map(|p| p.strip_prefix(&root).unwrap_or(p).display().to_string())
                    .collect();
                format!("- \"{}\": {}", cp.label, files.join(", "))
            })
            .collect();
        drop(guard);
        if lines.is_empty() {
            self.push_system("No checkpoints yet.");
        } else {
            self.push_system(format!(
                "Checkpoints (oldest first; /undo reverts the last):\n{}",
                lines.join("\n")
            ));
        }
    }

    fn compact_active_session(&mut self, ui_tx: mpsc::UnboundedSender<UiEvent>) {
        if self.is_busy {
            return;
        }
        let provider = match build_provider(&self.config) {
            Ok(p) => p,
            Err(err) => return self.push_error(err.to_string()),
        };
        let section = self.active_section;
        let session = Arc::clone(&self.active_state().session);
        let system_prompt = build_system_prompt(&self.config.workspace_dir);
        let window = self.config.context_window_tokens();
        let max_tokens = self.config.max_tokens;
        self.is_busy = true;
        self.push_system("Compacting conversation...");
        tokio::spawn(async move {
            let mut guard = session.lock().await;
            let event = if guard.is_empty() {
                AgentEvent::Notice("Nothing to compact.".into())
            } else {
                match compact_session(&provider, &mut guard, &system_prompt, window, max_tokens)
                    .await
                {
                    Ok(r) => AgentEvent::Notice(format!(
                        "Conversation compacted: ~{} -> ~{} tokens.",
                        r.before_tokens, r.after_tokens
                    )),
                    Err(err) => AgentEvent::Error(format!("Compaction failed: {err}")),
                }
            };
            drop(guard);
            let _ = ui_tx.send(UiEvent::Agent { section, event });
            let _ = ui_tx.send(UiEvent::Agent {
                section,
                event: AgentEvent::TurnComplete,
            });
        });
    }

    pub fn elapsed_secs(&self) -> f64 {
        self.started_at.elapsed().as_secs_f64()
    }

    pub fn is_landing_view(&self) -> bool {
        self.active_state().is_landing() && !self.is_busy
    }

    pub fn missing_key_hint(&self) -> Option<String> {
        if self.config.active_provider.requires_api_key()
            && self.config.active_settings().api_key.is_none()
        {
            let installed: Vec<&str> = self
                .detected_clis
                .iter()
                .filter(|c| c.path.is_some())
                .map(|c| c.kind.as_str())
                .collect();
            if !installed.is_empty() {
                let examples: Vec<String> = installed
                    .iter()
                    .take(3)
                    .map(|name| format!("/connect {name}"))
                    .collect();
                return Some(format!(
                    "Local CLIs ready: {}  (or set {})",
                    examples.join(" · "),
                    self.config
                        .active_provider
                        .env_key_name()
                        .unwrap_or("API_KEY")
                ));
            }
            Some(format!(
                "Set {} or switch with /provider ollama",
                self.config
                    .active_provider
                    .env_key_name()
                    .unwrap_or("API_KEY")
            ))
        } else {
            None
        }
    }

    pub fn context_usage(&self) -> (u32, u32, u8) {
        let max_ctx = self.config.context_window_tokens().max(1);
        let st = self.active_state();
        // Current conversation size, not the cumulative token counter, so the
        // bar drops after compaction. While a turn holds the lock, use the
        // prompt size reported by the last response.
        let used = match st.session.try_lock() {
            Ok(session) => crate::agent::compact::estimate_tokens("", session.messages())
                .max(session.last_prompt_tokens()),
            Err(_) => st.context_tokens,
        };
        let pct = ((used as u64 * 100) / max_ctx as u64).min(100) as u8;
        (used, max_ctx, pct)
    }

    pub fn active_swarm_domain(&self) -> Option<SwarmDomain> {
        match self.active_section {
            WorkspaceSection::Chat => None,
            WorkspaceSection::SwarmCode => Some(SwarmDomain::Code),
            WorkspaceSection::SwarmSecurity => Some(SwarmDomain::Security(self.security_sub_mode)),
            WorkspaceSection::SwarmTeam => Some(self.team_domain),
        }
    }

    pub fn modal_state(&self) -> ModalState {
        if let Some(ref ob) = self.onboarding {
            return match ob.step {
                OnboardingStep::Terms => ModalState::OnboardingTerms,
                OnboardingStep::Telemetry => ModalState::OnboardingTelemetry,
                OnboardingStep::Workspace if ob.manual_path_mode => {
                    ModalState::OnboardingWorkspaceInput
                }
                OnboardingStep::Workspace => ModalState::OnboardingWorkspace,
            };
        }
        if self.pending_approval.is_some() {
            ModalState::Approval
        } else if self.model_picker.is_some() {
            ModalState::ModelPicker
        } else if self.show_swarm_hub {
            ModalState::SwarmHub
        } else if self.show_help {
            ModalState::Help
        } else {
            ModalState::None
        }
    }

    pub fn prefetch_models(&self, ui_tx: mpsc::UnboundedSender<UiEvent>) {
        let cfg = self.config.clone();
        let provider = cfg.active_provider;
        tokio::spawn(async move {
            let models = fetch_models_for_config(&cfg).await;
            let _ = ui_tx.send(UiEvent::ModelsLoaded { provider, models });
        });
    }

    pub fn open_model_picker(&mut self, ui_tx: mpsc::UnboundedSender<UiEvent>) {
        if self.is_busy {
            return;
        }
        let provider = self.config.active_provider;
        let current = self.config.active_settings().model.clone();
        let (models, loading) = match self.model_cache.get(&provider) {
            Some(cached) if !cached.is_empty() => (cached.clone(), false),
            _ => {
                self.prefetch_models(ui_tx);
                (curated_fallback_models(provider), true)
            }
        };
        let selected = models.iter().position(|m| m.id == current).unwrap_or(0);
        self.show_swarm_hub = false;
        self.show_help = false;
        self.model_picker = Some(ModelPickerState {
            provider,
            models,
            filter: String::new(),
            selected,
            loading,
        });
    }

    pub fn refresh_model_picker(&mut self, ui_tx: mpsc::UnboundedSender<UiEvent>) {
        if let Some(ref mut picker) = self.model_picker {
            picker.loading = true;
        }
        self.prefetch_models(ui_tx);
    }

    pub fn model_picker_up(&mut self) {
        if let Some(ref mut picker) = self.model_picker {
            if picker.selected > 0 {
                picker.selected -= 1;
            }
        }
    }

    pub fn model_picker_down(&mut self) {
        if let Some(ref mut picker) = self.model_picker {
            let len = picker.filtered_models().len();
            if len > 0 && picker.selected + 1 < len {
                picker.selected += 1;
            }
        }
    }

    pub fn model_picker_char(&mut self, c: char) {
        if let Some(ref mut picker) = self.model_picker {
            picker.filter.push(c);
            picker.selected = 0;
        }
    }

    pub fn model_picker_backspace(&mut self) {
        if let Some(ref mut picker) = self.model_picker {
            picker.filter.pop();
            picker.selected = 0;
        }
    }

    pub fn model_picker_select(&mut self) {
        let chosen = self.model_picker.as_ref().and_then(|picker| {
            let filtered = picker.filtered_models();
            filtered.get(picker.selected).map(|m| m.id.clone())
        });
        if let Some(model_id) = chosen {
            self.config.set_model(model_id.clone());
            self.model_picker = None;
            if !self.is_landing_view() {
                let provider = self.config.active_provider;
                self.active_state_mut()
                    .entries
                    .push(ChatEntry::System(format!(
                        "Active model for {provider} set to '{model_id}'."
                    )));
                self.active_state_mut().auto_scroll = true;
            }
        }
    }

    pub fn open_full_setup(&mut self) {
        if self.is_busy {
            return;
        }
        self.onboarding = Some(OnboardingState::new_full(
            self.config.workspace_dir.clone(),
            self.prefs.telemetry_enabled,
            !self.prefs.needs_onboarding(),
        ));
    }

    pub fn open_folder_picker(&mut self) {
        if self.is_busy {
            return;
        }
        self.onboarding = Some(OnboardingState::new_folder_picker(
            self.config.workspace_dir.clone(),
            self.prefs.telemetry_enabled,
        ));
    }

    fn save_preferences(&self) {
        if let Some(ref path) = self.prefs_path {
            let _ = self.prefs.save_to(path);
        }
    }

    pub fn onboarding_confirm(&mut self) {
        let step = match self.onboarding.as_ref() {
            Some(ob) => ob.step,
            None => return,
        };

        match step {
            OnboardingStep::Terms => {
                self.prefs.accept_terms();
                if let Some(ref mut ob) = self.onboarding {
                    ob.step = OnboardingStep::Telemetry;
                }
                self.save_preferences();
            }
            OnboardingStep::Telemetry => {
                if let Some(ref mut ob) = self.onboarding {
                    self.prefs.telemetry_enabled = ob.telemetry_choice;
                    ob.step = OnboardingStep::Workspace;
                }
                self.save_preferences();
            }
            OnboardingStep::Workspace => {
                let result = self
                    .onboarding
                    .as_mut()
                    .map(|ob| ob.confirm_workspace())
                    .unwrap_or_else(|| Err(String::new()));
                match result {
                    Ok(chosen_dir) => {
                        self.config.workspace_dir = chosen_dir.clone();
                        self.prefs.record_workspace(chosen_dir.clone());
                        self.onboarding = None;
                        self.save_preferences();
                        if !self.is_landing_view() {
                            self.active_state_mut()
                                .entries
                                .push(ChatEntry::System(format!(
                                    "Workspace set to: {}",
                                    chosen_dir.display()
                                )));
                        }
                    }
                    Err(msg) => {
                        if !msg.is_empty() {
                            if let Some(ref mut ob) = self.onboarding {
                                ob.error_msg = Some(msg);
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn onboarding_cancel(&mut self) {
        let Some(ref ob) = self.onboarding else {
            return;
        };
        if ob.allow_cancel {
            self.onboarding = None;
        } else {
            self.should_quit = true;
        }
    }

    pub fn onboarding_up(&mut self) {
        if let Some(ref mut ob) = self.onboarding {
            ob.move_up();
        }
    }

    pub fn onboarding_down(&mut self) {
        if let Some(ref mut ob) = self.onboarding {
            ob.move_down();
        }
    }

    pub fn onboarding_open_subdir(&mut self) {
        if let Some(ref mut ob) = self.onboarding {
            ob.open_selected_subdir();
        }
    }

    pub fn onboarding_parent(&mut self) {
        if let Some(ref mut ob) = self.onboarding {
            ob.go_parent();
        }
    }

    pub fn onboarding_toggle_manual_path(&mut self) {
        if let Some(ref mut ob) = self.onboarding {
            ob.toggle_manual_path();
        }
    }

    pub fn onboarding_set_telemetry(&mut self, enabled: bool) {
        if let Some(ref mut ob) = self.onboarding {
            ob.telemetry_choice = enabled;
        }
    }

    pub fn onboarding_input_char(&mut self, c: char) {
        if let Some(ref mut ob) = self.onboarding {
            ob.error_msg = None;
            ob.manual_path_input.push(c);
        }
    }

    pub fn onboarding_input_backspace(&mut self) {
        if let Some(ref mut ob) = self.onboarding {
            ob.error_msg = None;
            ob.manual_path_input.pop();
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
            WorkspaceSection::SwarmTeam => &self.swarm_team_state,
        }
    }

    pub fn section_state_mut(&mut self, section: WorkspaceSection) -> &mut SectionState {
        match section {
            WorkspaceSection::Chat => &mut self.chat_state,
            WorkspaceSection::SwarmCode => &mut self.swarm_code_state,
            WorkspaceSection::SwarmSecurity => &mut self.swarm_security_state,
            WorkspaceSection::SwarmTeam => &mut self.swarm_team_state,
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

    pub fn set_team_domain(&mut self, domain: SwarmDomain) {
        if self.is_busy || !domain.is_team_mode() {
            return;
        }
        self.team_domain = domain;
        self.swarm_team_state.set_roster_for_domain(domain);
        self.swarm_team_state.entries.push(ChatEntry::System(format!(
            "{} -> {}",
            domain.display_name(),
            domain.subtitle()
        )));
        self.swarm_team_state.auto_scroll = true;
        self.active_section = WorkspaceSection::SwarmTeam;
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
        if !self.swarm_security_state.is_landing() {
            self.swarm_security_state
                .entries
                .push(ChatEntry::System(format!(
                    "Security mode -> {} ({})",
                    mode.display_name(),
                    domain.subtitle()
                )));
            self.swarm_security_state.auto_scroll = true;
        }
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

    pub fn trigger_quick_update(&mut self, ui_tx: mpsc::UnboundedSender<UiEvent>) {
        if self.is_busy {
            return;
        }
        self.is_busy = true;
        let target = self.available_update.as_ref().map(|u| u.tag_name.clone());
        self.active_state_mut().entries.push(ChatEntry::System(
            "Downloading and installing latest Ecliven Code release...".into(),
        ));
        self.active_state_mut().auto_scroll = true;
        tokio::spawn(async move {
            let res = updater::run_self_update(target.as_deref())
                .await
                .map_err(|e| e.to_string());
            let _ = ui_tx.send(UiEvent::UpdateFinished(res));
        });
    }

    pub fn close_modals(&mut self) {
        self.show_swarm_hub = false;
        self.show_help = false;
        self.model_picker = None;
    }

    pub fn set_scope(&mut self, raw: Option<&str>) {
        let Some(path_str) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            self.active_state_mut().target_scope = None;
            self.active_state_mut().entries.push(ChatEntry::System(
                "Target scope cleared (full workspace).".into(),
            ));
            return;
        };

        if matches!(
            path_str.to_ascii_lowercase().as_str(),
            "off" | "none" | "all" | "."
        ) {
            self.active_state_mut().target_scope = None;
            self.active_state_mut().entries.push(ChatEntry::System(
                "Target scope reset to full workspace.".into(),
            ));
            return;
        }

        let ctx = ToolContext::new(
            self.config.workspace_dir.clone(),
            self.config.shell_timeout_secs,
        );
        match ctx.resolve_path(path_str) {
            Ok(resolved) if resolved.exists() => {
                self.active_state_mut().target_scope = Some(path_str.to_string());
                self.active_state_mut()
                    .entries
                    .push(ChatEntry::System(format!(
                        "Target scope set to `{path_str}`."
                    )));
            }
            Ok(_) => {
                self.active_state_mut()
                    .entries
                    .push(ChatEntry::Error(format!(
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
        let st = self.active_state_mut();
        st.session.lock().await.clear();
        st.entries.clear();
        st.streaming_buffer.clear();
        st.reset_roster_idle();
        st.scroll_offset = 0;
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

        let is_swarm = self.active_swarm_domain().is_some();
        let selected = if is_swarm {
            build_provider(&self.config).map(|p| (p, None))
        } else {
            crate::ai::router::select_provider(&self.config, &prompt)
        };
        let provider = match selected {
            Ok((p, note)) => {
                if let Some(note) = note {
                    self.section_state_mut(section)
                        .entries
                        .push(ChatEntry::System(note));
                }
                p
            }
            Err(err) => {
                self.section_state_mut(section)
                    .entries
                    .push(ChatEntry::Error(err.to_string()));
                return;
            }
        };

        let tool_ctx = self.config.tool_context();
        let session = Arc::clone(&self.section_state(section).session);
        if let Some(store) = &self.session_store {
            let mut guard = session.lock().await;
            if guard.recorder().is_none() {
                if let Ok(rec) = store.create(
                    &self.config.workspace_dir,
                    self.config.active_provider.as_str(),
                    &self.config.active_settings().model,
                ) {
                    guard.attach_recorder(rec);
                }
            }
        }
        let prompt = if self.pending_notes.is_empty() {
            prompt
        } else {
            let notes = std::mem::take(&mut self.pending_notes).join("\n");
            format!("[Note: {notes}]\n\n{prompt}")
        };
        let target_scope = self.section_state(section).target_scope.clone();
        self.is_busy = true;
        self.section_state_mut(section).streaming_buffer.clear();

        if let Some(domain) = self.active_swarm_domain() {
            self.section_state_mut(section).reset_roster_idle();

            let mixed = crate::ai::router::swarm_providers(&self.config);
            for err in &mixed.errors {
                self.section_state_mut(section)
                    .entries
                    .push(ChatEntry::Error(format!("Swarm model skipped: {err}")));
            }
            let saver = self.config.saver_mode;
            let engine = SwarmEngine::new(
                domain,
                provider,
                Arc::clone(&self.tools),
                tool_ctx,
                SwarmOptions {
                    max_tokens: if saver {
                        self.config.max_tokens.min(2048)
                    } else {
                        self.config.max_tokens
                    },
                    temperature: self.config.temperature,
                    worker_max_iterations: if saver { 4 } else { 8 },
                    lead_max_iterations: self.config.max_tool_iterations,
                    approval_mode: self.config.approval_mode,
                    target_scope,
                    verify_command: self.config.swarm_verify_command.clone(),
                    verify_timeout_secs: self.config.swarm_verify_timeout_secs,
                    rounds: if saver { 1 } else { self.config.swarm_rounds },
                    saver,
                },
            )
            .with_providers(mixed.workers, mixed.lead);

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
            )
            .with_compaction(
                self.config.context_window_tokens(),
                self.config.effective_compact_percent(),
            )
            .with_saver_mode(self.config.saver_mode);
            let engine = match self.config.brief_options() {
                Some(opts) => engine.with_knowledge(opts, self.config.knowledge.workflow),
                None => engine,
            };

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
            UiEvent::McpConnected(conns) => {
                self.tools = Arc::new(ToolRegistry::with_extra_tools(&conns.tools));
                let summary = conns.summary();
                self.mcp = Some(conns);
                if !self.is_landing_view() {
                    self.push_system(summary);
                }
            }
            UiEvent::Swarm {
                section,
                domain,
                event,
            } => self.handle_swarm_event(section, domain, event),
            UiEvent::UpdateAvailable(info) => {
                let is_landing = self.is_landing_view();
                let msg = format!(
                    "Update available: v{} -> v{} ({}). Press F5 or run /update to upgrade.",
                    updater::CURRENT_VERSION,
                    info.version,
                    info.html_url
                );
                self.available_update = Some(info);
                if !is_landing {
                    self.active_state_mut().entries.push(ChatEntry::System(msg));
                    self.active_state_mut().auto_scroll = true;
                }
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
                        self.active_state_mut()
                            .entries
                            .push(ChatEntry::System(format!(
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
            UiEvent::ModelsLoaded { provider, models } => {
                self.model_cache.insert(provider, models.clone());
                if let Some(ref mut picker) = self.model_picker {
                    if picker.provider == provider {
                        let current = self.config.active_settings().model.clone();
                        picker.models = models;
                        picker.loading = false;
                        let filtered_len = picker.filtered_models().len();
                        if picker.filter.is_empty() {
                            picker.selected = picker
                                .models
                                .iter()
                                .position(|m| m.id == current)
                                .unwrap_or(0);
                        } else if picker.selected >= filtered_len {
                            picker.selected = filtered_len.saturating_sub(1);
                        }
                    }
                }
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
                if let Some(u) = &usage {
                    st.context_tokens = u.input_tokens;
                }
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
                    self.total_cache_read_tokens =
                        self.total_cache_read_tokens.saturating_add(u.cache_read_tokens);
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
            AgentEvent::Notice(msg) => {
                let st = self.section_state_mut(section);
                st.entries.push(ChatEntry::System(msg));
                st.auto_scroll = true;
            }
            AgentEvent::TurnComplete => {
                self.is_busy = false;
                let st = self.section_state_mut(section);
                st.streaming_buffer.clear();
                st.auto_scroll = true;
                let persist_error = st
                    .session
                    .try_lock()
                    .ok()
                    .and_then(|mut s| s.take_persist_error());
                if let Some(err) = persist_error {
                    st.entries.push(ChatEntry::Error(format!(
                        "Session could not be saved: {err}"
                    )));
                }
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
                    self.total_cache_read_tokens =
                        self.total_cache_read_tokens.saturating_add(u.cache_read_tokens);
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
                    self.total_cache_read_tokens =
                        self.total_cache_read_tokens.saturating_add(u.cache_read_tokens);
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
            SwarmEvent::Phase(text) => {
                let st = self.section_state_mut(section);
                st.entries.push(ChatEntry::System(text));
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

    async fn handle_slash_command(&mut self, raw: &str, ui_tx: mpsc::UnboundedSender<UiEvent>) {
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
                     /swarm competitive [folder]                     Candidates compete in isolated copies, tests pick the winner\n\
                     /swarm collaborative [folder]                   One agent per area: database, backend, frontend, security, testing\n\
                     /swarm red-eclipse [folder]                     Red team attacks, blue team hardens, verified rounds\n\
                     /swarm off  (or /chat)                          Return to standard single-agent Chat\n\
                     /scope <folder|off>                             Focus active Swarm on a specific folder/file\n\
                     /setup                                          Open setup wizard (Terms, Telemetry, Folder)\n\
                     /folder                                         Choose working folder (also F6)\n\
                     /telemetry <on|off|status>                      Enable or disable anonymous telemetry\n\
                     /update [check]                                 Check for updates or self-update Ecliven CLI\n\
                     /connect [codex|agy|claude|opencode|gemini]     Connect Ecliven to an installed AI CLI\n\
                     /connect custom <command> [args...]             Connect to any custom local AI CLI\n\
                     /provider <name>                                Switch API or CLI provider\n\
                     /model <model-name>                             Switch active model\n\
                     /approval <ask|auto-edit|auto-all>              Set tool approval policy\n\
                     /sandbox [read-only|workspace-write|off]        Show or set the run_command OS sandbox\n\
                     /compact                                        Summarize older history to free context\n\
                     /undo                                           Revert file changes from the last agent turn\n\
                     /checkpoints                                    List file checkpoints in this tab\n\
                     /sessions                                       List saved sessions for this workspace\n\
                     /resume [id|last]                               Resume a saved session in this tab\n\
                     /mcp                                            Show MCP servers and their tools\n\
                     /router [on|off]                                Auto-pick model by difficulty, cost, specialty\n\
                     /saver [on|off]                                 Saver mode: minimize tokens and cost\n\
                     /swarm-models                                   Show models used by Swarm agents\n\
                     /memory [search <q>|add <text>|forget <id>|confirm <id>|stale]  Project and user memory\n\
                     /index [rebuild]                                Refresh or rebuild the code index\n\
                     /tasks | /goal <text> | /goal done              Task history and active goal\n\
                     /workflow [on|off]                              Guided inspect-plan-verify-remember loop\n\
                     /trust [yes|deny|revoke|status]                 Enable executable settings of this workspace\n\
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
                    Some(team) => {
                        self.set_team_domain(team);
                        if let Some(scope) = arg2 {
                            self.set_scope(Some(scope));
                        }
                    }
                    None => {
                        self.active_state_mut().entries.push(ChatEntry::Error(format!(
                            "Unknown swarm domain '{domain_str}'. Use: /swarm code | competitive | collaborative | red-eclipse | security defense [folder] | security threat | off"
                        )));
                    }
                },
            },
            "/setup" => {
                self.open_full_setup();
            }
            "/folder" | "/dir" => {
                self.open_folder_picker();
            }
            "/telemetry" => match arg1.map(|s| s.to_ascii_lowercase()).as_deref() {
                Some("on" | "true" | "enable" | "activar") => {
                    self.prefs.telemetry_enabled = true;
                    self.save_preferences();
                    self.active_state_mut().entries.push(ChatEntry::System(
                        "Telemetry enabled (anonymous usage metrics only; no code or prompts)."
                            .into(),
                    ));
                }
                Some("off" | "false" | "disable" | "desactivar") => {
                    self.prefs.telemetry_enabled = false;
                    self.save_preferences();
                    self.active_state_mut().entries.push(ChatEntry::System(
                        "Telemetry disabled (zero data collection).".into(),
                    ));
                }
                _ => {
                    let state = if self.prefs.telemetry_enabled {
                        "enabled"
                    } else {
                        "disabled"
                    };
                    self.active_state_mut()
                        .entries
                        .push(ChatEntry::System(format!(
                            "Telemetry is currently {state}. Usage: /telemetry on | /telemetry off"
                        )));
                }
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
                    self.trigger_quick_update(ui_tx);
                }
            }
            "/chat" => {
                self.switch_section(WorkspaceSection::Chat);
            }
            "/connect" => {
                self.detected_clis = detect_installed_clis();
                match arg1 {
                    None => {
                        let mut lines = vec!["External CLI Backends:".to_string()];
                        for cli in &self.detected_clis {
                            let status = match &cli.path {
                                Some(p) => format!("installed ({})", p.display()),
                                None => "not found in PATH".to_string(),
                            };
                            let mark = if cli.path.is_some() { "✓" } else { "○" };
                            lines.push(format!("  {mark} {:<10} {}", cli.kind.as_str(), status));
                        }
                        lines.push(String::new());
                        lines.push("Connect with: /connect codex | /connect agy | /connect claude | /connect opencode | /connect custom <cmd>".to_string());
                        self.active_state_mut()
                            .entries
                            .push(ChatEntry::System(lines.join("\n")));
                    }
                    Some("custom") => {
                        let rest = raw
                            .split_once("custom")
                            .map(|(_, tail)| tail.trim())
                            .unwrap_or("");
                        if rest.is_empty() {
                            self.active_state_mut().entries.push(ChatEntry::Error(
                                "Usage: /connect custom <command> [args...]".into(),
                            ));
                        } else {
                            self.config.set_custom_cli_command(rest);
                            self.active_state_mut()
                                .entries
                                .push(ChatEntry::System(format!(
                                    "Connected to custom CLI command: `{rest}`."
                                )));
                        }
                    }
                    Some(target) => match ProviderKind::parse(target) {
                        Ok(kind) if kind.is_cli_backend() => {
                            let found = self
                                .detected_clis
                                .iter()
                                .find(|c| c.kind == kind)
                                .and_then(|c| c.path.as_ref());
                            self.config.set_provider(kind);
                            self.prefetch_models(ui_tx.clone());
                            let note = match found {
                                Some(p) => {
                                    format!("Connected to {} CLI ({}).", kind.as_str(), p.display())
                                }
                                None => format!(
                                    "Switched to {} CLI (warning: '{}' not found in PATH yet).",
                                    kind.as_str(),
                                    kind.default_base_url()
                                ),
                            };
                            self.active_state_mut()
                                .entries
                                .push(ChatEntry::System(note));
                        }
                        _ => {
                            self.active_state_mut().entries.push(ChatEntry::Error(format!(
                                "Unknown CLI '{target}'. Try: /connect codex | /connect agy | /connect claude | /connect opencode | /connect gemini | /connect custom <cmd>"
                            )));
                        }
                    },
                }
            }
            "/provider" => match arg1 {
                Some(name) => match ProviderKind::parse(name) {
                    Ok(kind) => {
                        self.config.set_provider(kind);
                        self.prefetch_models(ui_tx.clone());
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
            "/model" | "/models" => match arg1 {
                Some(model) => {
                    self.config.set_model_ref(model);
                    let msg = format!("Active model: {}", self.config.active_model_label());
                    self.active_state_mut().entries.push(ChatEntry::System(msg));
                }
                None => {
                    self.open_model_picker(ui_tx.clone());
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
                    self.active_state_mut().entries.push(ChatEntry::System(
                        "Approval mode set to 'auto-edit'.".into(),
                    ));
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
                let telemetry_desc = if self.prefs.telemetry_enabled {
                    "enabled"
                } else {
                    "disabled"
                };
                let msg = format!(
                    "Section:   {section_desc}\nScope:     {scope_desc}\nWorkspace: {}\nProvider:  {} ({})\nModel:     {}\nEndpoint:  {}\nApproval:  {:?}\nSandbox:   {}\nTelemetry: {telemetry_desc}\nTokens:    in={} out={} cached={} ({}% of input, prompt cache: {})",
                    self.config.workspace_dir.display(),
                    self.config.active_provider,
                    key_status,
                    settings.model,
                    settings.base_url,
                    self.config.approval_mode,
                    self.config.sandbox_policy().describe(),
                    self.total_input_tokens,
                    self.total_output_tokens,
                    self.total_cache_read_tokens,
                    (self.total_cache_read_tokens as u64 * 100)
                        .checked_div(self.total_input_tokens as u64)
                        .unwrap_or(0),
                    match self.config.prompt_cache {
                        crate::ai::PromptCache::Off => "off",
                        crate::ai::PromptCache::Short => "5m",
                        crate::ai::PromptCache::Long => "1h",
                    }
                );
                self.active_state_mut().entries.push(ChatEntry::System(msg));
            }
            "/clear" => {
                self.clear_session().await;
            }
            "/compact" => self.compact_active_session(ui_tx),
            "/undo" | "/revert" => self.undo_last_checkpoint().await,
            "/checkpoints" => self.list_checkpoints().await,
            "/memory" | "/index" | "/tasks" | "/goal" => {
                let ws = self.config.workspace_dir.clone();
                let name = cmd.to_string();
                let rest = raw.trim().strip_prefix(cmd).unwrap_or("").trim().to_string();
                let result = tokio::task::spawn_blocking(move || {
                    crate::knowledge::commands::run(&ws, &name, &rest)
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
                match result {
                    Ok(msg) => self.push_system(msg),
                    Err(e) => self.push_error(e),
                }
            }
            "/trust" => self.handle_trust_command(arg1, ui_tx.clone()),
            "/workflow" => {
                match arg1 {
                    Some("on") => self.config.knowledge.workflow = true,
                    Some("off") => self.config.knowledge.workflow = false,
                    Some(_) => return self.push_error("Usage: /workflow [on|off]"),
                    None => {}
                }
                let msg = format!(
                    "Workflow mode {}.",
                    if self.config.knowledge.workflow { "on" } else { "off" }
                );
                self.push_system(msg);
            }
            "/sessions" | "/history" => self.list_sessions(),
            "/resume" => self.resume_session(arg1.unwrap_or("last")).await,
            "/mcp" => {
                let msg = match &self.mcp {
                    Some(conns) => {
                        let tools: Vec<String> = self
                            .tools
                            .names()
                            .into_iter()
                            .filter(|n| n.starts_with("mcp__"))
                            .collect();
                        format!("{}\n{}", conns.summary(), tools.join("\n"))
                    }
                    None if self.config.mcp_servers.is_empty() => {
                        "No MCP servers configured. Add [mcp_servers.<name>] with `command` or `url` to .ecliven.toml.".into()
                    }
                    None => "MCP servers are still connecting...".into(),
                };
                self.push_system(msg);
            }
            "/sandbox" => match arg1 {
                None => {
                    let msg = format!("Sandbox: {}", self.config.sandbox_policy().describe());
                    self.push_system(msg);
                }
                Some(raw) => match SandboxMode::parse(raw) {
                    Some(mode) => {
                        self.config.sandbox_mode = mode;
                        let msg =
                            format!("Sandbox set to {}", self.config.sandbox_policy().describe());
                        self.push_system(msg);
                    }
                    None => self.push_error(
                        "Usage: /sandbox <read-only|workspace-write|danger-full-access>",
                    ),
                },
            },
            "/router" => {
                match arg1 {
                    Some("on") => self.config.router.enabled = true,
                    Some("off") => self.config.router.enabled = false,
                    Some(_) => return self.push_error("Usage: /router [on|off]"),
                    None => {}
                }
                let r = &self.config.router;
                let show = |v: &Option<String>| v.clone().unwrap_or_else(|| "-".into());
                let mut msg = format!(
                    "Router: {}\n  cheap: {}\n  standard: {}\n  strong: {}",
                    if r.enabled { "on" } else { "off" },
                    show(&r.cheap),
                    show(&r.standard),
                    show(&r.strong)
                );
                for (name, model) in &r.specialties {
                    msg.push_str(&format!("\n  {name}: {model}"));
                }
                if r.cheap.is_none() && r.standard.is_none() && r.strong.is_none() {
                    msg.push_str("\nConfigure [router] in .ecliven.toml with provider/model refs.");
                }
                self.push_system(msg);
            }
            "/saver" => {
                match arg1 {
                    Some("on") => self.config.saver_mode = true,
                    Some("off") => self.config.saver_mode = false,
                    Some(_) => return self.push_error("Usage: /saver [on|off]"),
                    None => {}
                }
                let msg = if self.config.saver_mode {
                    format!(
                        "Saver mode on: one model tier cheaper{}, max {} output tokens, tool output capped, compaction at {}%.",
                        self.config
                            .router
                            .cheap
                            .as_ref()
                            .map(|m| format!(" (cheap model {m})"))
                            .unwrap_or_default(),
                        self.config.max_tokens.min(2048),
                        self.config.effective_compact_percent()
                    )
                } else {
                    "Saver mode off.".to_string()
                };
                self.push_system(msg);
            }
            "/swarm-models" => {
                let workers = if self.config.swarm_worker_models.is_empty() {
                    "active model".to_string()
                } else {
                    self.config.swarm_worker_models.join(", ")
                };
                let lead = self
                    .config
                    .swarm_lead_model
                    .clone()
                    .unwrap_or_else(|| "active model".into());
                self.push_system(format!("Swarm workers: {workers}\nSwarm lead: {lead}"));
            }
            "/quit" | "/exit" => {
                self.should_quit = true;
            }
            other => {
                self.active_state_mut()
                    .entries
                    .push(ChatEntry::Error(format!(
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
