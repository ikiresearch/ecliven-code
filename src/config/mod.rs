pub mod defaults;
pub mod preferences;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use defaults::*;

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
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    provider: Option<ProviderKind>,
    model: Option<String>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    max_tool_iterations: Option<usize>,
    shell_timeout_secs: Option<u64>,
    approval_mode: Option<ApprovalMode>,
    providers: Option<FileProvidersConfig>,
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
        }
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
        let mut cfg = Self::default_for_workspace(workspace_dir);

        if let Some(path) = global_config_path {
            if path.exists() {
                cfg.apply_file(path)?;
            }
        }

        if let Some(path) = local_config_path {
            if path.exists() {
                cfg.apply_file(path)?;
            }
        }

        cfg.apply_env(env)?;
        cfg.apply_overrides(overrides);

        Ok(cfg)
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

    fn apply_file(&mut self, path: &Path) -> Result<(), ConfigError> {
        let raw = std::fs::read_to_string(path).map_err(|source| ConfigError::ReadFile {
            path: path.to_path_buf(),
            source,
        })?;
        let parsed: FileConfig = toml::from_str(&raw).map_err(|source| ConfigError::ParseToml {
            path: path.to_path_buf(),
            source: Box::new(source),
        })?;

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

        Ok(())
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
        assert_eq!(cfg.active_settings().api_key.as_deref(), Some("sk-test-123"));
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
