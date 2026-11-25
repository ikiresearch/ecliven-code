use std::io::{self, Write};
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tokio::sync::mpsc;

use ecliven_code::agent::session::Session;
use ecliven_code::agent::{AgentEngine, AgentEvent, AgentOptions};
use ecliven_code::ai::registry::build_provider;
use ecliven_code::cli::Cli;
use ecliven_code::config::AppConfig;
use ecliven_code::tools::{ToolContext, ToolRegistry};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let workspace_dir = match cli.dir {
        Some(ref dir) => dir.clone(),
        None => std::env::current_dir()?,
    };

    let config = AppConfig::load(workspace_dir, &cli.to_overrides())?;

    if let Some(prompt) = cli.prompt {
        run_headless(config, prompt).await
    } else {
        ecliven_code::tui::run_tui(config).await
    }
}

async fn run_headless(config: AppConfig, prompt: String) -> Result<()> {
    let provider = build_provider(&config)?;
    let engine = AgentEngine::new(
        provider,
        Arc::new(ToolRegistry::default_registry()),
        ToolContext::new(config.workspace_dir.clone(), config.shell_timeout_secs),
        AgentOptions {
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            max_tool_iterations: config.max_tool_iterations,
            approval_mode: config.approval_mode,
        },
    );

    let mut session = Session::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();

    let handle = tokio::spawn(async move {
        engine.run_turn(&mut session, prompt, tx).await;
    });

    while let Some(event) = rx.recv().await {
        match event {
            AgentEvent::AssistantDelta(chunk) => {
                print!("{chunk}");
                let _ = io::stdout().flush();
            }
            AgentEvent::AssistantFinished { .. } => {
                println!();
            }
            AgentEvent::ToolApprovalRequested {
                tool_name,
                preview,
                respond_to,
                ..
            } => {
                eprintln!("\n[approval] {}: {}", tool_name, preview.summary);
                if let Some(diff) = preview.diff {
                    eprintln!("{diff}");
                }
                eprint!("Approve? [y/N]: ");
                let _ = io::stderr().flush();

                let mut line = String::new();
                let _ = io::stdin().read_line(&mut line);
                let approved = matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes");
                let _ = respond_to.send(approved);
            }
            AgentEvent::ToolStarted { tool_name, summary } => {
                eprintln!("[tool:{tool_name}] {summary}");
            }
            AgentEvent::ToolCompleted { tool_name, output } => {
                if output.is_error {
                    eprintln!("[tool:{tool_name}:error] {}", output.content);
                }
            }
            AgentEvent::Error(err) => {
                eprintln!("Error: {err}");
            }
            AgentEvent::TurnComplete => break,
        }
    }

    let _ = handle.await;
    Ok(())
}
