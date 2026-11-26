pub mod defaults;
pub mod preferences;
pub mod trust;

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use defaults::*;

use crate::tools::sandbox::{SandboxMode, SandboxPolicy};

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error(
        "Unknown provider '{0}'. Supported: anthropic, openai, openrouter, ollama, codex, agy, claude, opencode, gemini, cli"
    )]
    UnknownProvider(String),

    #[error("Failed to read config file {path}: {source}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("Failed to parse TOML config {path}: {source}")]
    ParseToml {
        path: PathBuf,
        source: Box<toml::de::Error>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Anthropic,
    OpenAi,
    OpenRouter,
    Ollama,
    CodexCli,
    AgyCli,
    ClaudeCli,
    OpenCodeCli,
    GeminiCli,
    CustomCli,
}

impl ProviderKind {
    pub const ALL: [Self; 10] = [
        Self::Anthropic,
        Self::OpenAi,
        Self::OpenRouter,
        Self::Ollama,
        Self::CodexCli,
        Self::AgyCli,
        Self::ClaudeCli,
        Self::OpenCodeCli,
        Self::GeminiCli,
        Self::CustomCli,
    ];

    pub const EXTERNAL_CLIS: [Self; 5] = [
        Self::CodexCli,
        Self::AgyCli,
        Self::ClaudeCli,
        Self::OpenCodeCli,
        Self::GeminiCli,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::OpenRouter => "openrouter",
            Self::Ollama => "ollama",
            Self::CodexCli => "codex",
            Self::AgyCli => "agy",
            Self::ClaudeCli => "claude",
            Self::OpenCodeCli => "opencode",
            Self::GeminiCli => "gemini",
            Self::CustomCli => "cli",
        }
    }

    pub fn parse(input: &str) -> Result<Self, ConfigError> {
        match input.trim().to_ascii_lowercase().as_str() {
            "anthropic" => Ok(Self::Anthropic),
            "openai" => Ok(Self::OpenAi),
            "openrouter" => Ok(Self::OpenRouter),
            "ollama" | "local" => Ok(Self::Ollama),
            "codex" | "codex-cli" => Ok(Self::CodexCli),
            "agy" | "agy-cli" | "antigravity" => Ok(Self::AgyCli),
            "claude" | "claude-cli" | "claude-code" => Ok(Self::ClaudeCli),
            "opencode" | "opencode-cli" => Ok(Self::OpenCodeCli),
            "gemini" | "gemini-cli" => Ok(Self::GeminiCli),
            "cli" | "custom" | "custom-cli" => Ok(Self::CustomCli),
            other => Err(ConfigError::UnknownProvider(other.to_string())),
        }
    }

    pub fn default_model(&self) -> &'static str {
        match self {
            Self::Anthropic => DEFAULT_ANTHROPIC_MODEL,
            Self::OpenAi => DEFAULT_OPENAI_MODEL,
            Self::OpenRouter => DEFAULT_OPENROUTER_MODEL,
            Self::Ollama => DEFAULT_OLLAMA_MODEL,
            Self::CodexCli
            | Self::AgyCli
            | Self::ClaudeCli
            | Self::OpenCodeCli
            | Self::GeminiCli
            | Self::CustomCli => DEFAULT_CLI_MODEL,
        }
    }

    pub fn default_base_url(&self) -> &'static str {
        match self {
            Self::Anthropic => DEFAULT_ANTHROPIC_BASE_URL,
            Self::OpenAi => DEFAULT_OPENAI_BASE_URL,
            Self::OpenRouter => DEFAULT_OPENROUTER_BASE_URL,
            Self::Ollama => DEFAULT_OLLAMA_BASE_URL,
            Self::CodexCli => "codex",
            Self::AgyCli => "agy",
            Self::ClaudeCli => "claude",
            Self::OpenCodeCli => "opencode",
            Self::GeminiCli => "gemini",
            Self::CustomCli => "",
        }
    }

    pub fn is_cli_backend(&self) -> bool {
        matches!(
            self,
            Self::CodexCli
                | Self::AgyCli
                | Self::ClaudeCli
                | Self::OpenCodeCli
                | Self::GeminiCli
                | Self::CustomCli
        )
    }

    pub fn requires_api_key(&self) -> bool {
        matches!(self, Self::Anthropic | Self::OpenAi | Self::OpenRouter)
    }

    pub fn env_key_name(&self) -> Option<&'static str> {
        match self {
            Self::Anthropic => Some("ANTHROPIC_API_KEY"),
            Self::OpenAi => Some("OPENAI_API_KEY"),
            Self::OpenRouter => Some("OPENROUTER_API_KEY"),
            _ => None,
        }
    }

    pub fn context_window_tokens(&self) -> u32 {
        match self {
            Self::Anthropic | Self::ClaudeCli | Self::AgyCli => 200_000,
            Self::OpenAi | Self::OpenRouter | Self::CodexCli | Self::OpenCodeCli => 128_000,
            Self::GeminiCli => 1_000_000,
            Self::Ollama => 32_768,
            Self::CustomCli => 128_000,
        }
    }
}

impl fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalMode {
    #[default]
    Ask,
    AutoEdit,
    AutoAll,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ProviderSettings {
    pub model: String,
    pub base_url: String,
    #[serde(skip_serializing)]
    pub api_key: Option<String>,
}

impl fmt::Debug for ProviderSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderSettings")
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "***REDACTED***"))
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub workspace_dir: PathBuf,
    pub active_provider: ProviderKind,
    pub providers: HashMap<ProviderKind, ProviderSettings>,
    pub max_tokens: u32,
    pub temperature: f32,
    pub max_tool_iterations: usize,
    pub shell_timeout_secs: u64,
    pub approval_mode: ApprovalMode,
    pub sandbox_mode: SandboxMode,
    pub sandbox_network: bool,
    pub sandbox_writable_roots: Vec<PathBuf>,
    /// Compact the conversation when estimated context use reaches this percent (0 disables).
    pub auto_compact_percent: u8,
    /// Override of the provider's default context window in tokens.
    pub context_window: Option<u32>,
    pub mcp_servers: Vec<McpServerConfig>,
    pub router: crate::ai::router::RouterConfig,
    /// Minimize tokens and cost: cheaper models, shorter outputs, earlier compaction.
    pub saver_mode: bool,
    /// `provider/model` refs for Swarm workers (round-robin); empty = active model.
    pub swarm_worker_models: Vec<String>,
    pub swarm_lead_model: Option<String>,
    pub knowledge: KnowledgeConfig,
    pub workspace_trust: trust::WorkspaceTrust,
    pub cli_overrides: CliOverrides,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct KnowledgeConfig {
    /// Inject the project brief and expose knowledge tools.
    pub enabled: bool,
    /// Brief budget in tokens (default 1500, 600 in saver mode).
    pub brief_tokens: Option<usize>,
    /// Follow the inspect → plan → change → verify → review → report → remember loop.
    pub workflow: bool,
    /// `provider/model` used for semantic search (ollama, openai or openrouter).
    pub embeddings: Option<String>,
}

impl Default for KnowledgeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            brief_tokens: None,
            workflow: false,
            embeddings: None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct FileSwarmConfig {
    workers: Option<Vec<String>>,
    lead: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct McpServerConfig {
    #[serde(skip)]
    pub name: String,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub url: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Trusted servers run tools without approval prompts.
    #[serde(default)]
    pub trusted: bool,
    pub startup_timeout_secs: Option<u64>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    router: Option<crate::ai::router::RouterConfig>,
    saver_mode: Option<bool>,
    knowledge: Option<KnowledgeConfig>,
    swarm: Option<FileSwarmConfig>,
    sandbox_mode: Option<SandboxMode>,
    sandbox_network: Option<bool>,
    sandbox_writable_roots: Option<Vec<PathBuf>>,
    auto_compact_percent: Option<u8>,
    context_window: Option<u32>,
    mcp_servers: Option<std::collections::BTreeMap<String, McpServerConfig>>,
    provider: Option<ProviderKind>,
    model: Option<String>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    max_tool_iterations: Option<usize>,
    shell_timeout_secs: Option<u64>,
    approval_mode: Option<ApprovalMode>,
    providers: Option<FileProvidersConfig>,
}

impl FileConfig {
    /// Settings that can run code, weaken approvals or redirect API keys, as
    /// display lines plus the exact material the trust fingerprint covers.
    fn restricted_settings(&self) -> (Vec<String>, String) {
        let mut shown = Vec::new();
        let mut material = Vec::new();
        if let Some(mode) = self.approval_mode.filter(|m| *m != ApprovalMode::Ask) {
            let mode = serde_json::to_string(&mode).unwrap_or_default();
            shown.push(format!("approval_mode = {} (runs tools without asking)", mode.trim_matches('"')));
        }
        if self.sandbox_mode == Some(SandboxMode::DangerFullAccess) {
            shown.push("sandbox_mode = danger-full-access (commands run unsandboxed)".into());
        }
        if let Some(roots) = self.sandbox_writable_roots.as_ref().filter(|r| !r.is_empty()) {
            let list: Vec<String> = roots.iter().map(|r| r.display().to_string()).collect();
            shown.push(format!("sandbox_writable_roots = {}", list.join(", ")));
        }
        if let Some(servers) = &self.mcp_servers {
            for (name, s) in servers.iter().filter(|(_, s)| s.enabled) {
                let target = match (&s.command, &s.url) {
                    (Some(cmd), _) => format!("starts `{}`", [cmd.as_str()].into_iter().chain(s.args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ")),
                    (None, Some(url)) => format!("connects to {url}"),
                    (None, None) => "has no command or url".into(),
                };
                let mut line = format!("MCP server '{name}' {target}");
                if !s.env.is_empty() {
                    let keys: Vec<&str> = s.env.keys().map(String::as_str).collect();
                    line.push_str(&format!(" with env {}", keys.join(", ")));
                }
                if s.trusted {
                    line.push_str(" (tools run without approval)");
                }
                shown.push(line);
                let env: BTreeMap<_, _> = s.env.iter().collect();
                let headers: BTreeMap<_, _> = s.headers.iter().collect();
                material.push(format!("mcp:{name}:{env:?}:{headers:?}"));
            }
        }
        if let Some(p) = &self.providers {
            let entries = [
                ("anthropic", &p.anthropic),
                ("openai", &p.openai),
                ("openrouter", &p.openrouter),
                ("ollama", &p.ollama),
                ("codex", &p.codex),
                ("agy", &p.agy),
                ("claude", &p.claude),
                ("opencode", &p.opencode),
                ("gemini", &p.gemini),
                ("cli", &p.cli),
            ];
            for (name, entry) in entries {
                let Some(e) = entry else { continue };
                if let Some(cmd) = &e.command {
                    shown.push(format!("providers.{name}.command = {cmd} (runs a program)"));
                }
                if let Some(url) = &e.base_url {
                    shown.push(format!("providers.{name}.base_url = {url} (receives your API key and code)"));
                }
            }
        }
        material.extend(shown.iter().cloned());
        (shown, material.join("\n"))
    }

    fn withhold_restricted(&mut self) {
        if self.approval_mode != Some(ApprovalMode::Ask) {
            self.approval_mode = None;
        }
        if self.sandbox_mode == Some(SandboxMode::DangerFullAccess) {
            self.sandbox_mode = None;
        }
        self.sandbox_writable_roots = None;
        self.mcp_servers = None;
        if let Some(p) = &mut self.providers {
            for e in [
                &mut p.anthropic,
                &mut p.openai,
                &mut p.openrouter,
                &mut p.ollama,
                &mut p.codex,
                &mut p.agy,
                &mut p.claude,
                &mut p.opencode,
                &mut p.gemini,
                &mut p.cli,
            ]
            .into_iter()
            .flatten()
            {
                e.command = None;
                e.base_url = None;
            }
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct FileProvidersConfig {
    anthropic: Option<FileProviderEntry>,
    openai: Option<FileProviderEntry>,
    openrouter: Option<FileProviderEntry>,
    ollama: Option<FileProviderEntry>,
    codex: Option<FileProviderEntry>,
    agy: Option<FileProviderEntry>,
    claude: Option<FileProviderEntry>,
    opencode: Option<FileProviderEntry>,
    gemini: Option<FileProviderEntry>,
    cli: Option<FileProviderEntry>,
}

#[derive(Debug, Default, Deserialize)]
struct FileProviderEntry {
    model: Option<String>,
    base_url: Option<String>,
    command: Option<String>,
    api_key: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct CliOverrides {
    pub provider: Option<ProviderKind>,
    pub model: Option<String>,
    pub approval_mode: Option<ApprovalMode>,
    pub sandbox_mode: Option<SandboxMode>,
    pub saver_mode: bool,
    /// Trust the project's executable settings for this run without saving a decision.
    pub trust_workspace: bool,
}

impl AppConfig {
    pub fn default_for_workspace(workspace_dir: PathBuf) -> Self {
        let mut providers = HashMap::new();
        for kind in ProviderKind::ALL {
            providers.insert(
                kind,
                ProviderSettings {
                    model: kind.default_model().to_string(),
                    base_url: kind.default_base_url().to_string(),
                    api_key: None,
                },
            );
        }

        Self {
            workspace_dir,
            active_provider: ProviderKind::Anthropic,
            providers,
            max_tokens: DEFAULT_MAX_TOKENS,
            temperature: DEFAULT_TEMPERATURE,
            max_tool_iterations: DEFAULT_MAX_TOOL_ITERATIONS,
            shell_timeout_secs: DEFAULT_SHELL_TIMEOUT_SECS,
            approval_mode: ApprovalMode::Ask,
            sandbox_mode: SandboxMode::WorkspaceWrite,
            sandbox_network: true,
            sandbox_writable_roots: Vec::new(),
            auto_compact_percent: DEFAULT_AUTO_COMPACT_PERCENT,
            context_window: None,
            mcp_servers: Vec::new(),
            router: crate::ai::router::RouterConfig::default(),
            saver_mode: false,
            swarm_worker_models: Vec::new(),
            swarm_lead_model: None,
            knowledge: KnowledgeConfig::default(),
            workspace_trust: trust::WorkspaceTrust::default(),
            cli_overrides: CliOverrides::default(),
        }
    }

    /// Auto-compaction threshold, lowered in saver mode.
    pub fn effective_compact_percent(&self) -> u8 {
        if self.saver_mode && self.auto_compact_percent > 0 {
            self.auto_compact_percent.min(60)
        } else {
            self.auto_compact_percent
        }
    }

    pub fn sandbox_policy(&self) -> SandboxPolicy {
        SandboxPolicy::new(
            self.sandbox_mode,
            self.sandbox_network,
            self.sandbox_writable_roots.clone(),
        )
    }

    pub fn context_window_tokens(&self) -> u32 {
        self.context_window
            .unwrap_or_else(|| self.active_provider.context_window_tokens())
    }

    pub fn tool_context(&self) -> crate::tools::ToolContext {
        let embedding = self
            .knowledge
            .embeddings
            .as_deref()
            .and_then(|raw| crate::knowledge::EmbeddingClient::from_config(self, raw).ok());
        crate::tools::ToolContext::new(self.workspace_dir.clone(), self.shell_timeout_secs)
            .with_sandbox(self.sandbox_policy())
            .with_knowledge(crate::tools::KnowledgeSettings {
                saver: self.saver_mode,
                embedding,
            })
    }

    /// Brief options for the current mode, or `None` when knowledge is disabled.
    pub fn brief_options(&self) -> Option<crate::knowledge::BriefOptions> {
        if !self.knowledge.enabled {
            return None;
        }
        let mut opts = if self.saver_mode {
            crate::knowledge::BriefOptions::saver()
        } else {
            crate::knowledge::BriefOptions::standard()
        };
        if let Some(tokens) = self.knowledge.brief_tokens {
            opts.budget_tokens = if self.saver_mode { tokens.min(opts.budget_tokens) } else { tokens };
        }
        Some(opts)
    }

    pub fn load(workspace_dir: PathBuf, overrides: &CliOverrides) -> Result<Self, ConfigError> {
        let _ = dotenvy::dotenv();

        let global_path = dirs::config_dir().map(|d| d.join("ecliven-code").join("config.toml"));
        let local_path = workspace_dir.join(".ecliven.toml");

        let env_map: HashMap<String, String> = std::env::vars().collect();

        Self::load_from_sources(
            workspace_dir,
            global_path.as_deref(),
            Some(local_path.as_path()),
            &env_map,
            overrides,
        )
    }

    pub fn load_from_sources(
        workspace_dir: PathBuf,
        global_config_path: Option<&Path>,
        local_config_path: Option<&Path>,
        env: &HashMap<String, String>,
        overrides: &CliOverrides,
    ) -> Result<Self, ConfigError> {
        Self::load_from_sources_with_trust(
            workspace_dir,
            global_config_path,
            local_config_path,
            env,
            overrides,
            &trust::TrustStore::load_default(),
        )
    }

    /// The global config, env and CLI flags are the user's own and always apply;
    /// executable settings from the project config apply only if the workspace is trusted.
    pub fn load_from_sources_with_trust(
        workspace_dir: PathBuf,
        global_config_path: Option<&Path>,
        local_config_path: Option<&Path>,
        env: &HashMap<String, String>,
        overrides: &CliOverrides,
        trust_store: &trust::TrustStore,
    ) -> Result<Self, ConfigError> {
        let mut cfg = Self::default_for_workspace(workspace_dir);

        if let Some(path) = global_config_path {
            if path.exists() {
                cfg.apply_file(path)?;
            }
        }

        if let Some(path) = local_config_path {
            if path.exists() {
                let mut parsed = Self::parse_file(path)?;
                let (restricted, material) = parsed.restricted_settings();
                let key = trust::workspace_key(&cfg.workspace_dir);
                let fingerprint = crate::knowledge::hash_bytes(material.as_bytes());
                let status = if restricted.is_empty() {
                    trust::TrustStatus::NotRequired
                } else if overrides.trust_workspace {
                    trust::TrustStatus::Session
                } else {
                    trust_store.status(&key, &fingerprint)
                };
                cfg.workspace_trust = trust::WorkspaceTrust {
                    status,
                    key,
                    restricted,
                    fingerprint,
                };
                if cfg.workspace_trust.withheld() {
                    parsed.withhold_restricted();
                }
                cfg.apply_parsed(parsed);
            }
        }

        cfg.apply_env(env)?;
        cfg.apply_overrides(overrides);
        cfg.cli_overrides = overrides.clone();

        Ok(cfg)
    }

    /// Reloads from disk after a trust decision, keeping the CLI flags of this run.
    pub fn reload(&self) -> Result<Self, ConfigError> {
        Self::load(self.workspace_dir.clone(), &self.cli_overrides)
    }

    pub fn active_settings(&self) -> &ProviderSettings {
        self.providers
            .get(&self.active_provider)
            .expect("all built-in providers are initialized")
    }

    pub fn active_settings_mut(&mut self) -> &mut ProviderSettings {
        self.providers
            .get_mut(&self.active_provider)
            .expect("all built-in providers are initialized")
    }

    pub fn set_provider(&mut self, provider: ProviderKind) {
        self.active_provider = provider;
    }

    pub fn set_model(&mut self, model: impl Into<String>) {
        self.active_settings_mut().model = model.into();
    }

    pub fn set_custom_cli_command(&mut self, command: impl Into<String>) {
        self.active_provider = ProviderKind::CustomCli;
        self.providers
            .get_mut(&ProviderKind::CustomCli)
            .unwrap()
            .base_url = command.into();
    }

    fn parse_file(path: &Path) -> Result<FileConfig, ConfigError> {
        let raw = std::fs::read_to_string(path).map_err(|source| ConfigError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str(&raw).map_err(|source| ConfigError::ParseToml {
            path: path.to_path_buf(),
            source: Box::new(source),
        })
    }

    fn apply_file(&mut self, path: &Path) -> Result<(), ConfigError> {
        let parsed = Self::parse_file(path)?;
        self.apply_parsed(parsed);
        Ok(())
    }

    fn apply_parsed(&mut self, parsed: FileConfig) {

        if let Some(provider) = parsed.provider {
            self.active_provider = provider;
        }
        if let Some(max_tokens) = parsed.max_tokens {
            self.max_tokens = max_tokens;
        }
        if let Some(temperature) = parsed.temperature {
            self.temperature = temperature;
        }
        if let Some(max_iter) = parsed.max_tool_iterations {
            self.max_tool_iterations = max_iter;
        }
        if let Some(timeout) = parsed.shell_timeout_secs {
            self.shell_timeout_secs = timeout;
        }
        if let Some(approval) = parsed.approval_mode {
            self.approval_mode = approval;
        }
        if let Some(mode) = parsed.sandbox_mode {
            self.sandbox_mode = mode;
        }
        if let Some(network) = parsed.sandbox_network {
            self.sandbox_network = network;
        }
        if let Some(roots) = parsed.sandbox_writable_roots {
            self.sandbox_writable_roots = roots;
        }
        if let Some(pct) = parsed.auto_compact_percent {
            self.auto_compact_percent = pct.min(100);
        }
        if let Some(window) = parsed.context_window {
            self.context_window = Some(window);
        }
        if let Some(router) = parsed.router {
            self.router = router;
        }
        if let Some(knowledge) = parsed.knowledge {
            self.knowledge = knowledge;
        }
        if let Some(saver) = parsed.saver_mode {
            self.saver_mode = saver;
        }
        if let Some(swarm) = parsed.swarm {
            if let Some(workers) = swarm.workers {
                self.swarm_worker_models = workers;
            }
            if swarm.lead.is_some() {
                self.swarm_lead_model = swarm.lead;
            }
        }
        if let Some(servers) = parsed.mcp_servers {
            for (name, mut server) in servers {
                server.name = name;
                self.mcp_servers.retain(|s| s.name != server.name);
                self.mcp_servers.push(server);
            }
        }

        if let Some(providers) = parsed.providers {
            self.merge_provider_entry(ProviderKind::Anthropic, providers.anthropic);
            self.merge_provider_entry(ProviderKind::OpenAi, providers.openai);
            self.merge_provider_entry(ProviderKind::OpenRouter, providers.openrouter);
            self.merge_provider_entry(ProviderKind::Ollama, providers.ollama);
            self.merge_provider_entry(ProviderKind::CodexCli, providers.codex);
            self.merge_provider_entry(ProviderKind::AgyCli, providers.agy);
            self.merge_provider_entry(ProviderKind::ClaudeCli, providers.claude);
            self.merge_provider_entry(ProviderKind::OpenCodeCli, providers.opencode);
            self.merge_provider_entry(ProviderKind::GeminiCli, providers.gemini);
            self.merge_provider_entry(ProviderKind::CustomCli, providers.cli);
        }

        if let Some(model) = parsed.model {
            self.set_model(model);
        }
    }

    fn merge_provider_entry(&mut self, kind: ProviderKind, entry: Option<FileProviderEntry>) {
        if let Some(entry) = entry {
            if let Some(settings) = self.providers.get_mut(&kind) {
                if let Some(model) = entry.model {
                    settings.model = model;
                }
                if let Some(cmd) = entry.command {
                    settings.base_url = cmd;
                }
                if let Some(base_url) = entry.base_url {
                    settings.base_url = base_url;
                }
                if let Some(api_key) = entry.api_key {
                    if !api_key.trim().is_empty() {
                        settings.api_key = Some(api_key);
                    }
                }
            }
        }
    }

    fn apply_env(&mut self, env: &HashMap<String, String>) -> Result<(), ConfigError> {
        let non_empty = |key: &str| {
            env.get(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };

        if let Some(provider_str) = non_empty("ECLIVEN_PROVIDER") {
            self.active_provider = ProviderKind::parse(&provider_str)?;
        }

        if let Some(key) = non_empty("ANTHROPIC_API_KEY") {
            self.providers
                .get_mut(&ProviderKind::Anthropic)
                .unwrap()
                .api_key = Some(key);
        }
        if let Some(url) = non_empty("ANTHROPIC_BASE_URL") {
            self.providers
                .get_mut(&ProviderKind::Anthropic)
                .unwrap()
                .base_url = url;
        }

        if let Some(key) = non_empty("OPENAI_API_KEY") {
            self.providers
                .get_mut(&ProviderKind::OpenAi)
                .unwrap()
                .api_key = Some(key);
        }
        if let Some(url) = non_empty("OPENAI_BASE_URL") {
            self.providers
                .get_mut(&ProviderKind::OpenAi)
                .unwrap()
                .base_url = url;
        }

        if let Some(key) = non_empty("OPENROUTER_API_KEY") {
            self.providers
                .get_mut(&ProviderKind::OpenRouter)
                .unwrap()
                .api_key = Some(key);
        }

        if let Some(url) = non_empty("OLLAMA_BASE_URL") {
            self.providers
                .get_mut(&ProviderKind::Ollama)
                .unwrap()
                .base_url = url;
        }

        if let Some(cmd) = non_empty("ECLIVEN_CLI_CMD") {
            self.providers
                .get_mut(&ProviderKind::CustomCli)
                .unwrap()
                .base_url = cmd;
        }

        if let Some(model) = non_empty("ECLIVEN_MODEL") {
            self.set_model(model);
        }

        Ok(())
    }

    fn apply_overrides(&mut self, overrides: &CliOverrides) {
        if let Some(provider) = overrides.provider {
            self.active_provider = provider;
        }
        if let Some(ref model) = overrides.model {
            self.set_model(model.clone());
        }
        if let Some(mode) = overrides.approval_mode {
            self.approval_mode = mode;
        }
        if let Some(mode) = overrides.sandbox_mode {
            self.sandbox_mode = mode;
        }
        if overrides.saver_mode {
            self.saver_mode = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn loads_defaults_and_env_precedence() {
        let mut env = HashMap::new();
        env.insert("ECLIVEN_PROVIDER".into(), "openai".into());
        env.insert("OPENAI_API_KEY".into(), "sk-test-123".into());
        env.insert("ECLIVEN_MODEL".into(), "gpt-4.1-mini".into());

        let cfg = AppConfig::load_from_sources(
            PathBuf::from("."),
            None,
            None,
            &env,
            &CliOverrides::default(),
        )
        .unwrap();

        assert_eq!(cfg.active_provider, ProviderKind::OpenAi);
        assert_eq!(cfg.active_settings().model, "gpt-4.1-mini");
        assert_eq!(
            cfg.active_settings().api_key.as_deref(),
            Some("sk-test-123")
        );
    }

    #[test]
    fn parses_external_cli_providers_without_api_key_requirement() {
        for (raw, expected) in [
            ("codex", ProviderKind::CodexCli),
            ("agy", ProviderKind::AgyCli),
            ("claude", ProviderKind::ClaudeCli),
            ("opencode", ProviderKind::OpenCodeCli),
            ("gemini", ProviderKind::GeminiCli),
            ("cli", ProviderKind::CustomCli),
        ] {
            let parsed = ProviderKind::parse(raw).unwrap();
            assert_eq!(parsed, expected);
            assert!(parsed.is_cli_backend());
            assert!(!parsed.requires_api_key());
        }
    }

    #[test]
    fn cli_overrides_beat_file_and_env() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
provider = "ollama"
max_tokens = 2048

[providers.ollama]
model = "llama3.2"
"#
        )
        .unwrap();

        let mut env = HashMap::new();
        env.insert("ECLIVEN_PROVIDER".into(), "openai".into());

        let overrides = CliOverrides {
            provider: Some(ProviderKind::Ollama),
            model: Some("deepseek-coder:6.7b".into()),
            approval_mode: Some(ApprovalMode::AutoEdit),
            sandbox_mode: None,
            saver_mode: false,
            trust_workspace: false,
        };

        let cfg = AppConfig::load_from_sources(
            PathBuf::from("."),
            Some(file.path()),
            None,
            &env,
            &overrides,
        )
        .unwrap();

        assert_eq!(cfg.active_provider, ProviderKind::Ollama);
        assert_eq!(cfg.active_settings().model, "deepseek-coder:6.7b");
        assert_eq!(cfg.max_tokens, 2048);
        assert_eq!(cfg.approval_mode, ApprovalMode::AutoEdit);
    }

    #[test]
    fn parses_sandbox_compaction_and_mcp_servers() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            r#"
sandbox_mode = "read-only"
sandbox_network = false
auto_compact_percent = 70

saver_mode = true

[swarm]
workers = ["anthropic/test-a", "openai/test-b", "openrouter/test/c"]
lead = "anthropic/test-lead"

[router]
enabled = true
cheap = "ollama/test-small"

[knowledge]
workflow = true
brief_tokens = 900

[router.specialties]
security = "openrouter/test/sec"

[mcp_servers.docs]
command = "npx"
args = ["-y", "example-mcp"]

[mcp_servers.remote]
url = "https://example.com/mcp"
trusted = true
"#
        )
        .unwrap();

        let cfg = AppConfig::load_from_sources(
            PathBuf::from("."),
            Some(file.path()),
            None,
            &HashMap::new(),
            &CliOverrides::default(),
        )
        .unwrap();

        assert_eq!(cfg.sandbox_mode, SandboxMode::ReadOnly);
        assert!(!cfg.sandbox_network);
        assert_eq!(cfg.auto_compact_percent, 70);
        assert_eq!(cfg.effective_compact_percent(), 60);
        assert!(cfg.knowledge.enabled && cfg.knowledge.workflow);
        assert_eq!(cfg.brief_options().unwrap().budget_tokens, 600);
        assert!(cfg.saver_mode && cfg.router.enabled);
        assert_eq!(cfg.router.cheap.as_deref(), Some("ollama/test-small"));
        assert_eq!(cfg.router.specialties["security"], "openrouter/test/sec");
        assert_eq!(cfg.swarm_worker_models.len(), 3);
        assert_eq!(cfg.swarm_lead_model.as_deref(), Some("anthropic/test-lead"));
        assert_eq!(cfg.mcp_servers.len(), 2);
        let docs = cfg.mcp_servers.iter().find(|s| s.name == "docs").unwrap();
        assert_eq!(docs.command.as_deref(), Some("npx"));
        assert!(docs.enabled && !docs.trusted);
        assert!(cfg
            .mcp_servers
            .iter()
            .any(|s| s.name == "remote" && s.trusted));
    }

    #[test]
    fn provider_settings_redacts_api_key_in_debug() {
        let settings = ProviderSettings {
            model: "test".into(),
            base_url: "https://example.com".into(),
            api_key: Some("super-secret-token".into()),
        };
        let debug_out = format!("{settings:?}");
        assert!(!debug_out.contains("super-secret-token"));
        assert!(debug_out.contains("REDACTED"));
    }
}
