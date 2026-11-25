use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::config::{ApprovalMode, CliOverrides, ProviderKind};
use crate::swarm::domain::SwarmDomain;

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Check for a new release on GitHub and update the Ecliven CLI binary
    Update {
        /// Only check if a newer version is available without installing
        #[arg(long = "check")]
        check_only: bool,

        /// Install a specific release tag (e.g. v0.2.0)
        #[arg(long = "version")]
        version: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CliSwarmDomain {
    /// Multi-agent Code Swarm (3 parallel coders -> Master CHIEF combines best code)
    Code,
    /// Defensive Security Swarm — Defense mode (alias for security-defense)
    Security,
    /// Defensive Security Swarm — Code & folder vulnerability audit
    SecurityDefense,
    /// Defensive Security Swarm — STRIDE threat model, config/deps & hardening
    SecurityThreat,
}

impl From<CliSwarmDomain> for SwarmDomain {
    fn from(value: CliSwarmDomain) -> Self {
        match value {
            CliSwarmDomain::Code => SwarmDomain::Code,
            CliSwarmDomain::Security | CliSwarmDomain::SecurityDefense => {
                SwarmDomain::security_defense()
            }
            CliSwarmDomain::SecurityThreat => SwarmDomain::security_threat(),
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CliProvider {
    Anthropic,
    Openai,
    Openrouter,
    Ollama,
}

impl From<CliProvider> for ProviderKind {
    fn from(value: CliProvider) -> Self {
        match value {
            CliProvider::Anthropic => ProviderKind::Anthropic,
            CliProvider::Openai => ProviderKind::OpenAi,
            CliProvider::Openrouter => ProviderKind::OpenRouter,
            CliProvider::Ollama => ProviderKind::Ollama,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CliApprovalMode {
    Ask,
    AutoEdit,
    AutoAll,
}

impl From<CliApprovalMode> for ApprovalMode {
    fn from(value: CliApprovalMode) -> Self {
        match value {
            CliApprovalMode::Ask => ApprovalMode::Ask,
            CliApprovalMode::AutoEdit => ApprovalMode::AutoEdit,
            CliApprovalMode::AutoAll => ApprovalMode::AutoAll,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "ecliven",
    version,
    about = "Ecliven Code - Terminal-first AI coding agent in Rust"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Working directory for the coding session (defaults to current directory)
    #[arg(short = 'd', long = "dir")]
    pub dir: Option<PathBuf>,

    /// AI provider to use (anthropic, openai, openrouter, ollama)
    #[arg(short = 'p', long = "provider", value_enum)]
    pub provider: Option<CliProvider>,

    /// Model name override
    #[arg(short = 'm', long = "model")]
    pub model: Option<String>,

    /// Approval policy for mutating tools (ask, auto-edit, auto-all)
    #[arg(long = "approval", value_enum)]
    pub approval: Option<CliApprovalMode>,

    /// Start directly in Swarm mode (code, security, security-defense, security-threat)
    #[arg(short = 's', long = "swarm", value_enum)]
    pub swarm: Option<CliSwarmDomain>,

    /// Optional relative folder or file path inside the workspace to focus Swarm analysis on
    #[arg(long = "scope")]
    pub scope: Option<String>,

    /// Run a single non-interactive prompt and print the response to stdout
    #[arg(short = 'r', long = "prompt")]
    pub prompt: Option<String>,
}

impl Cli {
    pub fn to_overrides(&self) -> CliOverrides {
        CliOverrides {
            provider: self.provider.map(Into::into),
            model: self.model.clone(),
            approval_mode: self.approval.map(Into::into),
        }
    }
}
