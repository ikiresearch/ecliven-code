use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, Mutex};

use crate::agent::session::Session;
use crate::agent::{AgentEngine, AgentEvent, AgentOptions};
use crate::ai::registry::build_provider;
use crate::config::{AppConfig, ApprovalMode, ProviderKind};
use crate::tools::{ToolContext, ToolOutput, ToolPreview, ToolRegistry};

#[derive(Debug, Clone)]
pub enum ChatEntry {
    User(String),
    Assistant(String),
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

pub struct PendingApproval {
    pub tool_name: String,
    pub preview: ToolPreview,
    pub respond_to: oneshot::Sender<bool>,
}

pub struct App {
    pub config: AppConfig,
    pub session: Arc<Mutex<Session>>,
    pub entries: Vec<ChatEntry>,
    pub streaming_buffer: String,
    pub input: String,
    pub cursor_pos: usize,
    pub input_history: Vec<String>,
    pub history_index: Option<usize>,
    pub scroll_offset: u16,
    pub auto_scroll: bool,
    pub is_busy: bool,
    pub show_help: bool,
    pub should_quit: bool,
    pub tick_count: usize,
    pub total_input_tokens: u32,
    pub total_output_tokens: u32,
    pub pending_approval: Option<PendingApproval>,
    tools: Arc<ToolRegistry>,
}

impl App {
    pub fn new(config: AppConfig) -> Self {
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

        let entries = vec![ChatEntry::System(format!(
            "Welcome to Ecliven Code v{}. {provider_status}\nType a coding request or /help for commands.",
            env!("CARGO_PKG_VERSION")
        ))];

        Self {
            config,
            session: Arc::new(Mutex::new(Session::new())),
            entries,
            streaming_buffer: String::new(),
            input: String::new(),
            cursor_pos: 0,
            input_history: Vec::new(),
            history_index: None,
            scroll_offset: 0,
            auto_scroll: true,
            is_busy: false,
            show_help: false,
            should_quit: false,
            tick_count: 0,
            total_input_tokens: 0,
            total_output_tokens: 0,
            pending_approval: None,
            tools: Arc::new(ToolRegistry::default_registry()),
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
        self.auto_scroll = false;
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    pub fn scroll_down(&mut self, amount: u16) {
        self.scroll_offset = self.scroll_offset.saturating_add(amount);
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
        self.session.lock().await.clear();
        self.entries.clear();
        self.streaming_buffer.clear();
        self.total_input_tokens = 0;
        self.total_output_tokens = 0;
        self.entries
            .push(ChatEntry::System("Session cleared.".to_string()));
        self.auto_scroll = true;
    }

    pub async fn submit_input(&mut self, agent_tx: mpsc::UnboundedSender<AgentEvent>) {
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
        self.auto_scroll = true;

        if prompt.starts_with('/') {
            self.handle_slash_command(&prompt).await;
            return;
        }

        self.entries.push(ChatEntry::User(prompt.clone()));

        let provider = match build_provider(&self.config) {
            Ok(p) => p,
            Err(err) => {
                self.entries.push(ChatEntry::Error(err.to_string()));
                return;
            }
        };

        let engine = AgentEngine::new(
            provider,
            Arc::clone(&self.tools),
            ToolContext::new(
                self.config.workspace_dir.clone(),
                self.config.shell_timeout_secs,
            ),
            AgentOptions {
                max_tokens: self.config.max_tokens,
                temperature: self.config.temperature,
                max_tool_iterations: self.config.max_tool_iterations,
                approval_mode: self.config.approval_mode,
            },
        );

        let session = Arc::clone(&self.session);
        self.is_busy = true;
        self.streaming_buffer.clear();

        tokio::spawn(async move {
            let mut guard = session.lock().await;
            engine.run_turn(&mut guard, prompt, agent_tx).await;
        });
    }

    pub fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::AssistantDelta(chunk) => {
                self.streaming_buffer.push_str(&chunk);
                self.auto_scroll = true;
            }
            AgentEvent::AssistantFinished { content, usage } => {
                self.streaming_buffer.clear();
                if !content.trim().is_empty() {
                    self.entries.push(ChatEntry::Assistant(content));
                }
                if let Some(u) = usage {
                    self.total_input_tokens =
                        self.total_input_tokens.saturating_add(u.input_tokens);
                    self.total_output_tokens =
                        self.total_output_tokens.saturating_add(u.output_tokens);
                }
                self.auto_scroll = true;
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
                self.entries.push(ChatEntry::ToolCall {
                    name: tool_name,
                    summary,
                });
                self.auto_scroll = true;
            }
            AgentEvent::ToolCompleted {
                tool_name,
                output: ToolOutput { content, diff, is_error },
            } => {
                let preview_text = truncate_lines(&content, 24);
                self.entries.push(ChatEntry::ToolResult {
                    name: tool_name,
                    output: preview_text,
                    diff,
                    is_error,
                });
                self.auto_scroll = true;
            }
            AgentEvent::Error(err) => {
                self.streaming_buffer.clear();
                self.entries.push(ChatEntry::Error(err));
                self.auto_scroll = true;
            }
            AgentEvent::TurnComplete => {
                self.is_busy = false;
                self.streaming_buffer.clear();
                self.auto_scroll = true;
            }
        }
    }

    async fn handle_slash_command(&mut self, raw: &str) {
        let mut parts = raw.split_whitespace();
        let cmd = parts.next().unwrap_or("");
        let arg = parts.next();

        match cmd {
            "/help" => {
                self.entries.push(ChatEntry::System(
                    "Slash commands:\n\
                     /provider <anthropic|openai|openrouter|ollama>  Switch active AI provider\n\
                     /model <model-name>                             Switch active model\n\
                     /approval <ask|auto-edit|auto-all>              Set tool approval policy\n\
                     /status                                         Show session & provider status\n\
                     /clear                                          Clear chat history\n\
                     /quit                                           Exit Ecliven Code\n\n\
                     Shortcuts: Enter=Send | Ctrl+J=Newline | Ctrl+L=Clear | PgUp/PgDn=Scroll | F1=Help | Ctrl+Q=Quit"
                        .to_string(),
                ));
            }
            "/provider" => match arg {
                Some(name) => match ProviderKind::parse(name) {
                    Ok(kind) => {
                        self.config.set_provider(kind);
                        self.entries.push(ChatEntry::System(format!(
                            "Switched provider to {} (model: {}).",
                            kind,
                            self.config.active_settings().model
                        )));
                    }
                    Err(e) => self.entries.push(ChatEntry::Error(e.to_string())),
                },
                None => self.entries.push(ChatEntry::System(format!(
                    "Current provider: {}. Usage: /provider <anthropic|openai|openrouter|ollama>",
                    self.config.active_provider
                ))),
            },
            "/model" => match arg {
                Some(model) => {
                    self.config.set_model(model);
                    self.entries.push(ChatEntry::System(format!(
                        "Active model for {} set to '{}'.",
                        self.config.active_provider, model
                    )));
                }
                None => self.entries.push(ChatEntry::System(format!(
                    "Current model: {}. Usage: /model <model-name>",
                    self.config.active_settings().model
                ))),
            },
            "/approval" => match arg {
                Some("ask") => {
                    self.config.approval_mode = ApprovalMode::Ask;
                    self.entries
                        .push(ChatEntry::System("Approval mode set to 'ask'.".into()));
                }
                Some("auto-edit") => {
                    self.config.approval_mode = ApprovalMode::AutoEdit;
                    self.entries
                        .push(ChatEntry::System("Approval mode set to 'auto-edit'.".into()));
                }
                Some("auto-all") => {
                    self.config.approval_mode = ApprovalMode::AutoAll;
                    self.entries
                        .push(ChatEntry::System("Approval mode set to 'auto-all'.".into()));
                }
                _ => self.entries.push(ChatEntry::System(format!(
                    "Current approval mode: {:?}. Usage: /approval <ask|auto-edit|auto-all>",
                    self.config.approval_mode
                ))),
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
                self.entries.push(ChatEntry::System(format!(
                    "Workspace: {}\nProvider:  {} ({})\nModel:     {}\nEndpoint:  {}\nApproval:  {:?}\nTokens:    in={} out={}",
                    self.config.workspace_dir.display(),
                    self.config.active_provider,
                    key_status,
                    settings.model,
                    settings.base_url,
                    self.config.approval_mode,
                    self.total_input_tokens,
                    self.total_output_tokens
                )));
            }
            "/clear" => {
                self.clear_session().await;
            }
            "/quit" | "/exit" => {
                self.should_quit = true;
            }
            other => {
                self.entries.push(ChatEntry::Error(format!(
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
