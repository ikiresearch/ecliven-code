use std::path::PathBuf;

use clap::{Parser, ValueEnum};

use crate::config::{ApprovalMode, CliOverrides, ProviderKind};

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
