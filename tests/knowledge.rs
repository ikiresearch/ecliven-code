use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::mpsc;

use ecliven_code::agent::session::Session;
use ecliven_code::agent::{AgentEngine, AgentEvent, AgentOptions};
use ecliven_code::ai::{AiError, CompletionRequest, CompletionResponse, LlmProvider, ToolCall};
use ecliven_code::config::ApprovalMode;
use ecliven_code::knowledge::{BriefOptions, KnowledgeStore};
use ecliven_code::tools::{ToolContext, ToolRegistry};

struct Recording {
    turns: Mutex<Vec<CompletionResponse>>,
    seen: Mutex<Vec<String>>,
}

#[async_trait]
impl LlmProvider for Recording {
    fn name(&self) -> &'static str {
        "recording"
    }

    fn model(&self) -> &str {
        "test/model"
    }

    async fn complete(
        &self,
        request: &CompletionRequest,
        _delta_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<CompletionResponse, AiError> {
        let last = request.messages.last().map(|m| m.content.clone()).unwrap_or_default();
        self.seen.lock().unwrap().push(last);
        Ok(self.turns.lock().unwrap().remove(0))
    }
}

fn reply(content: &str, tool: Option<(&str, serde_json::Value)>) -> CompletionResponse {
    CompletionResponse {
        content: content.into(),
        tool_calls: tool
            .map(|(name, arguments)| {
                vec![ToolCall {
                    id: "call_1".into(),
                    name: name.into(),
                    arguments,
                }]
            })
            .unwrap_or_default(),
        usage: None,
        stop_reason: None,
        external_tools: Vec::new(),
    }
}

#[tokio::test]
async fn turn_gets_brief_uses_knowledge_tools_and_is_recorded() {
    let data = TempDir::new().unwrap();
    std::env::set_var("ECLIVEN_KNOWLEDGE_DIR", data.path());
    let ws = TempDir::new().unwrap();
    std::fs::create_dir_all(ws.path().join("src")).unwrap();
    std::fs::write(ws.path().join("src/rate.rs"), "pub fn RateLimiter_new() {}\npub struct RateLimiter;\n").unwrap();
    {
        let mut store = KnowledgeStore::open(ws.path()).unwrap();
        store.refresh().unwrap();
        store
            .add_memory("project", "convention", "RateLimiter must be shared per tenant", &["src/rate.rs".into()])
            .unwrap();
    }

    let provider = Arc::new(Recording {
        turns: Mutex::new(vec![
            reply("", Some(("find_symbol", json!({ "name": "RateLimiter" })))),
            reply("", Some(("memory_save", json!({ "text": "RateLimiter lives in src/rate.rs", "sources": ["src/rate.rs"] })))),
            reply("Done: RateLimiter is in src/rate.rs.", None),
        ]),
        seen: Mutex::new(Vec::new()),
    });
    let engine = AgentEngine::new(
        provider.clone(),
        Arc::new(ToolRegistry::default_registry()),
        ToolContext::new(ws.path().to_path_buf(), 10),
        AgentOptions {
            max_tokens: 512,
            temperature: 0.0,
            max_tool_iterations: 5,
            approval_mode: ApprovalMode::AutoAll,
        },
    )
    .with_knowledge(BriefOptions::saver(), true);

    let mut session = Session::new();
    let (tx, mut rx) = mpsc::unbounded_channel();
    engine.run_turn(&mut session, "Where is RateLimiter defined?".into(), tx).await;
    let mut tool_output = String::new();
    while let Ok(ev) = rx.try_recv() {
        if let AgentEvent::ToolCompleted { output, .. } = ev {
            tool_output.push_str(&output.content);
        }
    }

    let seen = provider.seen.lock().unwrap();
    assert!(seen[0].starts_with("Where is RateLimiter defined?"));
    assert!(seen[0].contains("<project_context>"));
    assert!(seen[0].contains("RateLimiter must be shared per tenant (src: src/rate.rs)"));
    assert!(tool_output.contains("def src/rate.rs:2 struct"), "{tool_output}");

    let store = KnowledgeStore::open(ws.path()).unwrap();
    assert_eq!(store.search_memories("RateLimiter lives", 5).unwrap().len(), 2);
    let tasks = store.recent_tasks(5).unwrap();
    assert_eq!(tasks[0].goal, "Where is RateLimiter defined?");
    assert_eq!(tasks[0].status, "done");
}
