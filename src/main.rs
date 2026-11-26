use std::io::{self, Write};
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tokio::sync::mpsc;

use ecliven_code::agent::session::Session;
use ecliven_code::agent::store::{self, SessionStore};
use ecliven_code::agent::{AgentEngine, AgentEvent, AgentOptions};
use ecliven_code::ai::registry::build_provider;
use ecliven_code::cli::{Cli, Commands};
use ecliven_code::config::AppConfig;
use ecliven_code::mcp;
use ecliven_code::swarm::domain::SwarmDomain;
use ecliven_code::swarm::{SwarmEngine, SwarmEvent, SwarmOptions};
use ecliven_code::tools::ToolRegistry;
use ecliven_code::updater::{self, UpdateStatus};

#[tokio::main]
async fn main() -> Result<()> {
    let raw_args: Vec<String> = std::env::args().collect();
    if raw_args.get(1).map(String::as_str) == Some(ecliven_code::tools::sandbox::SANDBOX_SUBCOMMAND)
    {
        ecliven_code::tools::sandbox::sandbox_main(&raw_args[2..]);
    }

    updater::cleanup_stale_backup();

    let cli = Cli::parse();
    let workspace_dir = match cli.dir {
        Some(ref dir) => dir.clone(),
        None => std::env::current_dir()?,
    };

    match cli.command {
        Some(Commands::Update {
            check_only,
            version,
        }) => {
            return run_update_command(check_only, version).await;
        }
        Some(Commands::Models) => {
            let config = AppConfig::load(workspace_dir, &cli.to_overrides())?;
            let models = ecliven_code::ai::models::fetch_models_for_config(&config).await;
            println!(
                "Available models for {} (active: {}):",
                config.active_provider,
                config.active_settings().model
            );
            for m in models {
                let marker = if m.id == config.active_settings().model {
                    "*"
                } else {
                    " "
                };
                if m.label == m.id {
                    println!("  {marker} {}", m.id);
                } else {
                    println!("  {marker} {:<32} {}", m.id, m.label);
                }
            }
            return Ok(());
        }
        Some(Commands::Sessions { all }) => {
            let config = AppConfig::load(workspace_dir, &cli.to_overrides())?;
            let Some(store) = SessionStore::default_store() else {
                anyhow::bail!("No data directory available for sessions");
            };
            let filter = (!all).then_some(config.workspace_dir.as_path());
            let sessions = store.list(filter);
            if sessions.is_empty() {
                println!(
                    "No saved sessions{}.",
                    if all { "" } else { " for this workspace" }
                );
            }
            for s in sessions {
                println!(
                    "{}  {:>9}  {:>3} msgs  {}{}",
                    s.meta.id,
                    store::format_age(s.updated_at),
                    s.message_count,
                    s.meta.title,
                    if all {
                        format!("  ({})", s.meta.workspace.display())
                    } else {
                        String::new()
                    }
                );
            }
            return Ok(());
        }
        Some(Commands::Mcp) => {
            let config = AppConfig::load(workspace_dir, &cli.to_overrides())?;
            let conns = mcp::connect_all(&config.mcp_servers).await;
            println!("{}", conns.summary());
            for tool in &conns.tools {
                println!("    {}", tool.name());
            }
            return Ok(());
        }
        Some(Commands::Resume { .. }) | None => {}
    }

    let resume = match &cli.command {
        Some(Commands::Resume { id }) => Some(id.clone().unwrap_or_else(|| "last".into())),
        _ => cli.resume.clone(),
    };
    let config = AppConfig::load(workspace_dir, &cli.to_overrides())?;
    let initial_swarm: Option<SwarmDomain> = cli.swarm.map(Into::into);
    let initial_scope = cli.scope;

    if let Some(prompt) = cli.prompt {
        let update_task = tokio::spawn(updater::check_for_update());
        let res = if let Some(domain) = initial_swarm {
            run_headless_swarm(config, domain, initial_scope, prompt).await
        } else {
            run_headless(config, prompt, resume).await
        };
        if let Ok(Ok(Ok(UpdateStatus::Available { current, latest }))) =
            tokio::time::timeout(std::time::Duration::from_millis(500), update_task).await
        {
            eprintln!(
                "\n[ecliven] v{current} is outdated -> v{} available! Run 'ecliven update' to upgrade.",
                latest.version
            );
        }
        res
    } else {
        ecliven_code::tui::run_tui(config, initial_swarm, initial_scope, cli.setup, resume).await
    }
}

async fn run_headless(config: AppConfig, prompt: String, resume: Option<String>) -> Result<()> {
    let (provider, note) = ecliven_code::ai::router::select_provider(&config, &prompt)?;
    if let Some(note) = note {
        eprintln!("[ecliven] {note}");
    }
    let conns = mcp::connect_all(&config.mcp_servers).await;
    for err in &conns.errors {
        eprintln!("[mcp] {err}");
    }
    let engine = AgentEngine::new(
        provider,
        Arc::new(ToolRegistry::with_extra_tools(&conns.tools)),
        config.tool_context(),
        AgentOptions {
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            max_tool_iterations: config.max_tool_iterations,
            approval_mode: config.approval_mode,
        },
    )
    .with_compaction(
        config.context_window_tokens(),
        config.effective_compact_percent(),
    )
    .with_saver_mode(config.saver_mode);

    let mut session = Session::new();
    if let Some(store) = SessionStore::default_store() {
        match resume {
            Some(query) => {
                let found = store
                    .find(&query, &config.workspace_dir)
                    .map_err(anyhow::Error::msg)?;
                let loaded = store::load_session(&found.path).map_err(anyhow::Error::msg)?;
                session = Session::from_loaded(loaded);
                session.attach_recorder(store.reopen(&found));
                eprintln!("[ecliven] Resumed session {}", found.meta.id);
            }
            None => {
                if let Ok(rec) = store.create(
                    &config.workspace_dir,
                    config.active_provider.as_str(),
                    &config.active_settings().model,
                ) {
                    session.attach_recorder(rec);
                }
            }
        }
    }
    let session_id = session.recorder().map(|r| r.id().to_string());
    let (tx, mut rx) = mpsc::unbounded_channel::<AgentEvent>();

    let handle = tokio::spawn(async move {
        engine.run_turn(&mut session, prompt, tx).await;
        session
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
            AgentEvent::Notice(msg) => {
                eprintln!("[ecliven] {msg}");
            }
            AgentEvent::TurnComplete => break,
        }
    }

    let session = handle.await.ok();
    if let Some(err) = session.and_then(|mut s| s.take_persist_error()) {
        eprintln!("[ecliven] Warning: session could not be saved: {err}");
    }
    if let Some(id) = session_id {
        eprintln!("[ecliven] Session {id} saved. Continue with: ecliven resume {id}");
    }
    Ok(())
}

async fn run_headless_swarm(
    config: AppConfig,
    domain: SwarmDomain,
    target_scope: Option<String>,
    prompt: String,
) -> Result<()> {
    let provider = build_provider(&config)?;
    let mixed = ecliven_code::ai::router::swarm_providers(&config);
    for err in &mixed.errors {
        eprintln!("[swarm] model skipped: {err}");
    }
    let saver = config.saver_mode;
    let engine = SwarmEngine::new(
        domain,
        provider,
        Arc::new(ToolRegistry::default_registry()),
        config.tool_context(),
        SwarmOptions {
            max_tokens: if saver {
                config.max_tokens.min(2048)
            } else {
                config.max_tokens
            },
            temperature: config.temperature,
            worker_max_iterations: if saver { 4 } else { 8 },
            lead_max_iterations: config.max_tool_iterations,
            approval_mode: config.approval_mode,
            target_scope,
        },
    )
    .with_providers(mixed.workers, mixed.lead);

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
