//! Conversation compaction: replaces older history with an LLM-written handoff
//! summary when the context window fills up.

use std::sync::Arc;

use crate::agent::session::Session;
use crate::ai::{AiError, CompletionRequest, LlmProvider, Message, Role};

pub const SUMMARY_PREFIX: &str = "[Context compacted] Summary of the earlier conversation (written by a previous model run; use it to continue without redoing work):\n";

const COMPACT_PROMPT: &str =
    "You are performing a CONTEXT CHECKPOINT COMPACTION for a coding agent. \
Write a handoff summary that lets another model continue the work seamlessly. Include:\n\
- The user's goals, constraints and preferences\n\
- Progress so far and key decisions, with file paths, commands and results that matter\n\
- Files created or modified and their current state\n\
- Open problems and clear next steps\n\
Be concise and structured. Do not invent anything that is not in the transcript.";

const MAX_TOOL_OUTPUT_CHARS: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionPolicy {
    pub context_window: u32,
    pub threshold_percent: u8,
}

impl CompactionPolicy {
    pub fn should_compact(&self, used_tokens: u32) -> bool {
        self.threshold_percent > 0
            && self.context_window > 0
            && used_tokens as u64 * 100
                >= self.context_window as u64 * self.threshold_percent as u64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionResult {
    pub before_tokens: u32,
    pub after_tokens: u32,
}

pub fn estimate_tokens(system_prompt: &str, messages: &[Message]) -> u32 {
    let chars: usize = system_prompt.len()
        + messages
            .iter()
            .map(|m| {
                m.content.len()
                    + m.tool_calls
                        .iter()
                        .map(|c| c.name.len() + c.arguments.to_string().len())
                        .sum::<usize>()
            })
            .sum::<usize>();
    (chars / 4) as u32
}

fn render_transcript(messages: &[Message], max_chars: usize) -> String {
    let mut out = String::new();
    for m in messages {
        let label = match m.role {
            Role::System => "System",
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::Tool => "Tool result",
        };
        let mut body = m.content.clone();
        if m.role == Role::Tool && body.len() > MAX_TOOL_OUTPUT_CHARS {
            let mut cut = MAX_TOOL_OUTPUT_CHARS;
            while !body.is_char_boundary(cut) {
                cut -= 1;
            }
            body.truncate(cut);
            body.push_str(" ... [truncated]");
        }
        out.push_str(label);
        if let Some(name) = &m.tool_name {
            out.push_str(&format!(" ({name})"));
        }
        out.push_str(":\n");
        out.push_str(&body);
        for call in &m.tool_calls {
            out.push_str(&format!("\n[tool call] {} {}", call.name, call.arguments));
        }
        out.push_str("\n\n");
    }
    if out.len() > max_chars {
        let mut start = out.len() - max_chars;
        while !out.is_char_boundary(start) {
            start += 1;
        }
        out = format!(
            "[earliest part of the transcript omitted]\n{}",
            &out[start..]
        );
    }
    out
}

/// Summarizes everything before the latest user message and keeps that message
/// (and the tool exchange after it) verbatim.
pub async fn compact_session(
    provider: &Arc<dyn LlmProvider>,
    session: &mut Session,
    system_prompt: &str,
    context_window: u32,
    max_tokens: u32,
) -> Result<CompactionResult, AiError> {
    let before_tokens = estimate_tokens(system_prompt, session.messages());
    let messages = session.messages().to_vec();
    let split = messages
        .iter()
        .rposition(|m| m.role == Role::User)
        .unwrap_or(0);
    let (head, tail) = if split == 0 {
        (messages.as_slice(), &[][..])
    } else {
        messages.split_at(split)
    };
    if head.is_empty() {
        return Ok(CompactionResult {
            before_tokens,
            after_tokens: before_tokens,
        });
    }

    let max_chars = (context_window as usize).saturating_mul(3).max(8_000);
    let request = CompletionRequest {
        system_prompt: COMPACT_PROMPT.to_string(),
        messages: vec![Message::user(format!(
            "Transcript to summarize:\n\n{}",
            render_transcript(head, max_chars)
        ))],
        tools: Vec::new(),
        max_tokens,
        temperature: 0.0,
    };
    let response = provider.complete(&request, None).await?;
    let summary = response.content.trim();
    if summary.is_empty() {
        return Err(AiError::Parse(
            "compaction returned an empty summary".into(),
        ));
    }
    let summary_block = format!("{SUMMARY_PREFIX}{summary}");

    let new_messages = if tail.is_empty() {
        vec![Message::user(format!(
            "{summary_block}\n\nContinue with the task."
        ))]
    } else {
        let mut kept = tail.to_vec();
        kept[0].content = format!("{summary_block}\n\n---\n\n{}", kept[0].content);
        kept
    };
    session.replace_messages(new_messages);
    Ok(CompactionResult {
        before_tokens,
        after_tokens: estimate_tokens(system_prompt, session.messages()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{CompletionResponse, ToolCall};
    use async_trait::async_trait;
    use serde_json::json;
    use tokio::sync::mpsc;

    struct Summarizer;

    #[async_trait]
    impl LlmProvider for Summarizer {
        fn name(&self) -> &'static str {
            "summarizer"
        }
        fn model(&self) -> &str {
            "test/model"
        }
        async fn complete(
            &self,
            request: &CompletionRequest,
            _delta_tx: Option<mpsc::UnboundedSender<String>>,
        ) -> Result<CompletionResponse, AiError> {
            assert!(request.tools.is_empty());
            assert!(request.messages[0].content.contains("first question"));
            Ok(CompletionResponse {
                content: "User asked about main.rs; it prints hello.".into(),
                ..CompletionResponse::default()
            })
        }
    }

    #[tokio::test]
    async fn keeps_latest_user_turn_and_summarizes_the_rest() {
        let mut session = Session::new();
        session.push_user("first question");
        session.push_assistant(
            "reading",
            vec![ToolCall {
                id: "c1".into(),
                name: "read_file".into(),
                arguments: json!({"path": "main.rs"}),
            }],
        );
        session.push_tool_result(
            "c1",
            "read_file",
            "fn main() { println!(\"hello\"); }".repeat(50),
        );
        session.push_assistant("It prints hello.", vec![]);
        session.push_user("second question");

        let provider: Arc<dyn LlmProvider> = Arc::new(Summarizer);
        let result = compact_session(&provider, &mut session, "sys", 100_000, 512)
            .await
            .unwrap();

        assert_eq!(session.len(), 1);
        let only = &session.messages()[0];
        assert_eq!(only.role, Role::User);
        assert!(only.content.starts_with(SUMMARY_PREFIX));
        assert!(only.content.ends_with("second question"));
        assert!(result.after_tokens < result.before_tokens);
    }

    #[test]
    fn policy_threshold() {
        let p = CompactionPolicy {
            context_window: 1000,
            threshold_percent: 80,
        };
        assert!(!p.should_compact(799));
        assert!(p.should_compact(800));
        assert!(!CompactionPolicy {
            threshold_percent: 0,
            ..p
        }
        .should_compact(5000));
    }
}
