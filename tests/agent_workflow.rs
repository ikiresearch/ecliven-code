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
use ecliven_code::tools::{ToolContext, ToolRegistry};
use ecliven_code::tui::app::App;

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
    let mut app = App::new(cfg);

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
