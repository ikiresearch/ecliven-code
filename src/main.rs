use std::io::{self, Write};
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tokio::sync::mpsc;

use ecliven_code::agent::session::Session;
use ecliven_code::agent::{AgentEngine, AgentEvent, AgentOptions};
use ecliven_code::ai::registry::build_provider;
use ecliven_code::cli::{Cli, Commands};
use ecliven_code::config::AppConfig;
use ecliven_code::swarm::domain::SwarmDomain;
use ecliven_code::swarm::{SwarmEngine, SwarmEvent, SwarmOptions};
use ecliven_code::tools::{ToolContext, ToolRegistry};
use ecliven_code::updater::{self, UpdateStatus};

#[tokio::main]
async fn main() -> Result<()> {
    updater::cleanup_stale_backup();

    let cli = Cli::parse();
    if let Some(Commands::Update {
        check_only,
        version,
    }) = cli.command
    {
        return run_update_command(check_only, version).await;
    }

    let workspace_dir = match cli.dir {
        Some(ref dir) => dir.clone(),
        None => std::env::current_dir()?,
    };

    let config = AppConfig::load(workspace_dir, &cli.to_overrides())?;
    let initial_swarm: Option<SwarmDomain> = cli.swarm.map(Into::into);
    let initial_scope = cli.scope;

    if let Some(prompt) = cli.prompt {
        if let Some(domain) = initial_swarm {
            run_headless_swarm(config, domain, initial_scope, prompt).await
        } else {
            run_headless(config, prompt).await
        }
    } else {
        ecliven_code::tui::run_tui(config, initial_swarm, initial_scope).await
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
                let approved = prompt_cli_approval(&tool_name, &preview.summary, preview.diff);
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

async fn run_headless_swarm(
    config: AppConfig,
    domain: SwarmDomain,
    target_scope: Option<String>,
    prompt: String,
) -> Result<()> {
    let provider = build_provider(&config)?;
    let engine = SwarmEngine::new(
        domain,
        provider,
        Arc::new(ToolRegistry::default_registry()),
        ToolContext::new(config.workspace_dir.clone(), config.shell_timeout_secs),
        SwarmOptions {
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            worker_max_iterations: 8,
            lead_max_iterations: config.max_tool_iterations,
            approval_mode: config.approval_mode,
            target_scope,
        },
    );

    let mut session = Session::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<SwarmEvent>();

    let handle = tokio::spawn(async move {
        engine.run_turn(&mut session, prompt, tx).await;
    });

    while let Some(event) = rx.recv().await {
        match event {
            SwarmEvent::SwarmStarted { domain, scope } => {
                let suffix = scope.map(|s| format!(" (scope: {s})")).unwrap_or_default();
                eprintln!("[swarm] Starting {}{suffix}...", domain.display_name());
            }
            SwarmEvent::WorkerReport {
                badge,
                agent_name,
                content,
                ..
            } => {
                eprintln!("\n--- [{badge}] {agent_name} ---\n{content}");
            }
            SwarmEvent::Synthesizing { lead_badge } => {
                eprintln!("\n=== [{lead_badge}] Synthesizing ===");
            }
            SwarmEvent::SynthesisDelta(chunk) => {
                print!("{chunk}");
                let _ = io::stdout().flush();
            }
            SwarmEvent::SynthesisFinished { .. } => {
                println!();
            }
            SwarmEvent::ToolApprovalRequested {
                tool_name,
                preview,
                respond_to,
                ..
            } => {
                let approved = prompt_cli_approval(&tool_name, &preview.summary, preview.diff);
                let _ = respond_to.send(approved);
            }
            SwarmEvent::ToolStarted { tool_name, summary } => {
                eprintln!("[tool:{tool_name}] {summary}");
            }
            SwarmEvent::ToolCompleted { tool_name, output } => {
                if output.is_error {
                    eprintln!("[tool:{tool_name}:error] {}", output.content);
                }
            }
            SwarmEvent::Error(err) => {
                eprintln!("Error: {err}");
            }
            SwarmEvent::SwarmComplete => break,
            SwarmEvent::WorkerStatus { .. } => {}
        }
    }

    let _ = handle.await;
    Ok(())
}

fn prompt_cli_approval(tool_name: &str, summary: &str, diff: Option<String>) -> bool {
    eprintln!("\n[approval] {tool_name}: {summary}");
    if let Some(diff_text) = diff {
        eprintln!("{diff_text}");
    }
    eprint!("Approve? [y/N]: ");
    let _ = io::stderr().flush();

    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);
    matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

async fn run_update_command(check_only: bool, version: Option<String>) -> Result<()> {
    if let Some(ref pinned) = version {
        println!("Installing Ecliven Code {pinned}...");
        let output = updater::run_self_update(Some(pinned)).await?;
        println!("{output}");
        return Ok(());
    }

    println!(
        "Checking for updates (current: v{})...",
        updater::CURRENT_VERSION
    );
    match updater::check_for_update().await? {
        UpdateStatus::UpToDate { current } => {
            println!("Ecliven Code v{current} is already up to date.");
        }
        UpdateStatus::Available { current, latest } => {
            println!(
                "New update available: v{current} -> v{} ({})",
                latest.version, latest.html_url
            );
            if check_only {
                println!("Run 'ecliven update' to install it.");
            } else {
                println!("Updating Ecliven Code to v{}...", latest.version);
                let output = updater::run_self_update(Some(&latest.tag_name)).await?;
                println!("{output}");
            }
        }
    }

    Ok(())
}
