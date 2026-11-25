use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::mpsc;

use ecliven_code::agent::session::Session;
use ecliven_code::agent::{AgentEngine, AgentEvent, AgentOptions};
use ecliven_code::ai::{
    AiError, CompletionRequest, CompletionResponse, LlmProvider, TokenUsage, ToolCall,
};
use ecliven_code::config::{AppConfig, ApprovalMode, ProviderKind};
use ecliven_code::swarm::domain::{SecuritySubMode, SwarmDomain};
use ecliven_code::swarm::{SwarmEngine, SwarmEvent, SwarmOptions};
use ecliven_code::tools::{ToolContext, ToolRegistry};
use ecliven_code::tui::app::{App, ChatEntry, WorkspaceSection};

struct ScriptedProvider {
    turns: Mutex<Vec<CompletionResponse>>,
}

impl ScriptedProvider {
    fn new(turns: Vec<CompletionResponse>) -> Self {
        Self {
            turns: Mutex::new(turns),
        }
    }
}

#[async_trait]
impl LlmProvider for ScriptedProvider {
    fn name(&self) -> &'static str {
        "scripted"
    }

    fn model(&self) -> &str {
        "scripted-v1"
    }

    async fn complete(
        &self,
        _request: &CompletionRequest,
        delta_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<CompletionResponse, AiError> {
        let resp = self.turns.lock().unwrap().remove(0);
        if let Some(tx) = delta_tx {
            if !resp.content.is_empty() {
                let _ = tx.send(resp.content.clone());
            }
        }
        Ok(resp)
    }
}

#[tokio::test]
async fn edit_file_requires_approval_in_ask_mode_and_applies_when_approved() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("app.rs");
    std::fs::write(&target, "fn main() { println!(\"v1\"); }\n").unwrap();

    let provider = Arc::new(ScriptedProvider::new(vec![
        CompletionResponse {
            content: "Updating version string.".into(),
            tool_calls: vec![ToolCall {
                id: "call_1".into(),
                name: "edit_file".into(),
                arguments: json!({
                    "path": "app.rs",
                    "old_string": "v1",
                    "new_string": "v2"
                }),
            }],
            usage: Some(TokenUsage {
                input_tokens: 15,
                output_tokens: 10,
            }),
            stop_reason: Some("tool_use".into()),
        },
        CompletionResponse {
            content: "Updated app.rs to v2.".into(),
            tool_calls: Vec::new(),
            usage: Some(TokenUsage {
                input_tokens: 25,
                output_tokens: 8,
            }),
            stop_reason: Some("end_turn".into()),
        },
    ]));

    let engine = AgentEngine::new(
        provider,
        Arc::new(ToolRegistry::default_registry()),
        ToolContext::new(tmp.path().to_path_buf(), 10),
        AgentOptions {
            max_tokens: 1024,
            temperature: 0.0,
            max_tool_iterations: 5,
            approval_mode: ApprovalMode::Ask,
        },
    );

    let (tx, mut rx) = mpsc::unbounded_channel();
    let handle = tokio::spawn(async move {
        let mut session = Session::new();
        engine
            .run_turn(&mut session, "Bump version to v2".into(), tx)
            .await;
        session
    });

    let mut saw_approval_with_diff = false;
    while let Some(ev) = rx.recv().await {
        match ev {
            AgentEvent::ToolApprovalRequested {
                tool_name,
                preview,
                respond_to,
                ..
            } => {
                assert_eq!(tool_name, "edit_file");
                let diff = preview.diff.expect("edit_file preview should include diff");
                assert!(diff.contains("-fn main() { println!(\"v1\"); }"));
                assert!(diff.contains("+fn main() { println!(\"v2\"); }"));
                saw_approval_with_diff = true;
                respond_to.send(true).unwrap();
            }
            AgentEvent::TurnComplete => break,
            _ => {}
        }
    }

    let session = handle.await.unwrap();
    assert!(saw_approval_with_diff);
    assert!(std::fs::read_to_string(&target).unwrap().contains("v2"));
    assert_eq!(session.messages().last().unwrap().content, "Updated app.rs to v2.");
}

#[tokio::test]
async fn denied_file_write_does_not_modify_disk() {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join("secret.txt");
    std::fs::write(&target, "original").unwrap();

    let provider = Arc::new(ScriptedProvider::new(vec![
        CompletionResponse {
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: "call_deny".into(),
                name: "write_file".into(),
                arguments: json!({
                    "path": "secret.txt",
                    "content": "overwritten"
                }),
            }],
            usage: None,
            stop_reason: Some("tool_use".into()),
        },
        CompletionResponse {
            content: "Understood, I did not touch secret.txt.".into(),
            tool_calls: Vec::new(),
            usage: None,
            stop_reason: Some("end_turn".into()),
        },
    ]));

    let engine = AgentEngine::new(
        provider,
        Arc::new(ToolRegistry::default_registry()),
        ToolContext::new(tmp.path().to_path_buf(), 10),
        AgentOptions {
            max_tokens: 512,
            temperature: 0.0,
            max_tool_iterations: 5,
            approval_mode: ApprovalMode::Ask,
        },
    );

    let (tx, mut rx) = mpsc::unbounded_channel();
    let handle = tokio::spawn(async move {
        let mut session = Session::new();
        engine.run_turn(&mut session, "Overwrite secret.txt".into(), tx).await;
    });

    while let Some(ev) = rx.recv().await {
        match ev {
            AgentEvent::ToolApprovalRequested { respond_to, .. } => {
                respond_to.send(false).unwrap();
            }
            AgentEvent::TurnComplete => break,
            _ => {}
        }
    }

    handle.await.unwrap();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
}

#[tokio::test]
async fn tui_app_slash_commands_switch_provider_and_model() {
    let tmp = TempDir::new().unwrap();
    let cfg = AppConfig::default_for_workspace(tmp.path().to_path_buf());
    let mut app = App::new(cfg, None, None);

    let (tx, _rx) = mpsc::unbounded_channel();

    app.input = "/provider ollama".into();
    app.cursor_pos = app.input.len();
    app.submit_input(tx.clone()).await;
    assert_eq!(app.config.active_provider, ProviderKind::Ollama);

    app.input = "/model qwen2.5-coder:7b".into();
    app.cursor_pos = app.input.len();
    app.submit_input(tx.clone()).await;
    assert_eq!(app.config.active_settings().model, "qwen2.5-coder:7b");

    app.input = "/approval auto-edit".into();
    app.cursor_pos = app.input.len();
    app.submit_input(tx).await;
    assert_eq!(app.config.approval_mode, ApprovalMode::AutoEdit);
}

#[tokio::test]
async fn tui_swarm_sections_and_security_submodes_stay_isolated() {
    let tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(tmp.path().join("src/auth")).unwrap();
    let cfg = AppConfig::default_for_workspace(tmp.path().to_path_buf());
    let mut app = App::new(cfg, None, None);
    let (tx, _rx) = mpsc::unbounded_channel();

    assert_eq!(app.active_section, WorkspaceSection::Chat);

    app.input = "/swarm".into();
    app.cursor_pos = app.input.len();
    app.submit_input(tx.clone()).await;
    assert!(app.show_swarm_hub);

    app.input = "/swarm code".into();
    app.cursor_pos = app.input.len();
    app.submit_input(tx.clone()).await;
    assert_eq!(app.active_section, WorkspaceSection::SwarmCode);
    assert!(!app.show_swarm_hub);
    assert_eq!(app.active_state().roster.len(), 3);
    assert_eq!(app.active_state().roster[0].badge, "CLEAN");

    app.active_state_mut()
        .entries
        .push(ChatEntry::User("Implement fast parser".into()));

    app.input = "/swarm security defense src/auth".into();
    app.cursor_pos = app.input.len();
    app.submit_input(tx.clone()).await;
    assert_eq!(app.active_section, WorkspaceSection::SwarmSecurity);
    assert_eq!(app.security_sub_mode, SecuritySubMode::Defense);
    assert_eq!(app.active_state().target_scope.as_deref(), Some("src/auth"));
    assert_eq!(app.active_state().roster[0].badge, "SURFACE");

    assert!(
        !app.active_state()
            .entries
            .iter()
            .any(|e| matches!(e, ChatEntry::User(s) if s == "Implement fast parser")),
        "Security swarm transcript must remain isolated from Code swarm transcript"
    );

    app.input = "/swarm security threat".into();
    app.cursor_pos = app.input.len();
    app.submit_input(tx.clone()).await;
    assert_eq!(app.security_sub_mode, SecuritySubMode::ThreatModel);
    assert_eq!(app.active_state().roster[0].badge, "STRIDE");

    app.input = "/scope off".into();
    app.cursor_pos = app.input.len();
    app.submit_input(tx.clone()).await;
    assert_eq!(app.active_state().target_scope, None);

    app.input = "/swarm off".into();
    app.cursor_pos = app.input.len();
    app.submit_input(tx).await;
    assert_eq!(app.active_section, WorkspaceSection::Chat);
}

struct DomainMockProvider;

#[async_trait]
impl LlmProvider for DomainMockProvider {
    fn name(&self) -> &'static str {
        "domain-mock"
    }

    fn model(&self) -> &str {
        "domain-mock-1"
    }

    async fn complete(
        &self,
        request: &CompletionRequest,
        delta_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<CompletionResponse, AiError> {
        let content: String = if request.system_prompt.contains("Master Combiner Agent") {
            "CHIEF combined the clean API from CLEAN, zero-copy buffer from PERF, and edge-case tests from ROBUST.".into()
        } else if request.system_prompt.contains("Clean Architecture Specialist") {
            "CLEAN candidate implementation.".into()
        } else if request.system_prompt.contains("Performance & Systems Specialist") {
            "PERF candidate implementation.".into()
        } else {
            "ROBUST candidate implementation with tests.".into()
        };

        if let Some(tx) = delta_tx {
            let _ = tx.send(content.clone());
        }

        Ok(CompletionResponse {
            content,
            tool_calls: Vec::new(),
            usage: Some(TokenUsage {
                input_tokens: 12,
                output_tokens: 6,
            }),
            stop_reason: Some("end_turn".into()),
        })
    }
}

#[tokio::test]
async fn code_swarm_runs_three_coders_on_same_task_and_chief_combines_best_code() {
    let tmp = TempDir::new().unwrap();
    let engine = SwarmEngine::new(
        SwarmDomain::Code,
        Arc::new(DomainMockProvider),
        Arc::new(ToolRegistry::default_registry()),
        ToolContext::new(tmp.path().to_path_buf(), 10),
        SwarmOptions::default(),
    );

    let mut session = Session::new();
    let (tx, mut rx) = mpsc::unbounded_channel();

    engine
        .run_turn(&mut session, "Build a fast LRU cache".into(), tx)
        .await;

    let mut badges = Vec::new();
    let mut chief_done = false;
    while let Some(ev) = rx.recv().await {
        match ev {
            SwarmEvent::WorkerReport { badge, .. } => badges.push(badge),
            SwarmEvent::SynthesisFinished {
                lead_badge,
                content,
                ..
            } => {
                assert_eq!(lead_badge, "CHIEF");
                assert!(content.contains("CHIEF combined"));
                chief_done = true;
            }
            SwarmEvent::SwarmComplete => break,
            _ => {}
        }
    }

    badges.sort();
    assert_eq!(badges, vec!["CLEAN", "PERF", "ROBUST"]);
    assert!(chief_done);
}
