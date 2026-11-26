use std::path::PathBuf;

use crate::agent::checkpoint::{Checkpoint, MAX_CHECKPOINTS};
use crate::agent::store::SessionRecorder;
use crate::ai::{Message, TokenUsage, ToolCall};

#[derive(Debug, Default, Clone)]
pub struct Session {
    messages: Vec<Message>,
    total_usage: TokenUsage,
    last_prompt_tokens: u32,
    checkpoints: Vec<Checkpoint>,
    recorder: Option<SessionRecorder>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_messages(messages: Vec<Message>) -> Self {
        Self {
            messages,
            ..Self::default()
        }
    }

    pub fn attach_recorder(&mut self, recorder: SessionRecorder) {
        self.recorder = Some(recorder);
    }

    pub fn recorder(&self) -> Option<&SessionRecorder> {
        self.recorder.as_ref()
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn total_usage(&self) -> &TokenUsage {
        &self.total_usage
    }

    /// Input tokens reported for the latest request (0 if unknown or after compaction).
    pub fn last_prompt_tokens(&self) -> u32 {
        self.last_prompt_tokens
    }

    fn push(&mut self, message: Message) {
        if let Some(rec) = self.recorder.as_mut() {
            rec.record_message(&message);
        }
        self.messages.push(message);
    }

    pub fn push_user(&mut self, content: impl Into<String>) {
        self.push(Message::user(content));
    }

    pub fn push_assistant(&mut self, content: impl Into<String>, tool_calls: Vec<ToolCall>) {
        self.push(Message::assistant(content, tool_calls));
    }

    pub fn push_tool_result(
        &mut self,
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        output: impl Into<String>,
    ) {
        self.push(Message::tool_result(tool_call_id, tool_name, output));
    }

    /// Replaces the history (used by compaction) and records it.
    pub fn replace_messages(&mut self, messages: Vec<Message>) {
        if let Some(rec) = self.recorder.as_mut() {
            rec.record_replace(&messages);
        }
        self.messages = messages;
        self.last_prompt_tokens = 0;
    }

    pub fn add_usage(&mut self, usage: &TokenUsage) {
        self.total_usage.input_tokens = self
            .total_usage
            .input_tokens
            .saturating_add(usage.input_tokens);
        self.total_usage.output_tokens = self
            .total_usage
            .output_tokens
            .saturating_add(usage.output_tokens);
        self.last_prompt_tokens = usage.input_tokens;
    }

    /// Starts the checkpoint for a new user turn (reusing an unused one).
    pub fn begin_checkpoint(&mut self, label: impl Into<String>) {
        if self.checkpoints.last().is_some_and(Checkpoint::is_empty) {
            self.checkpoints.pop();
        }
        self.checkpoints.push(Checkpoint::new(label));
        if self.checkpoints.len() > MAX_CHECKPOINTS {
            self.checkpoints.remove(0);
        }
    }

    pub fn capture_before_write(&mut self, paths: &[PathBuf]) {
        if self.checkpoints.is_empty() {
            self.checkpoints.push(Checkpoint::new("changes"));
        }
        let cp = self.checkpoints.last_mut().expect("checkpoint exists");
        for p in paths {
            cp.capture(p);
        }
    }

    pub fn checkpoints(&self) -> impl Iterator<Item = &Checkpoint> {
        self.checkpoints.iter().filter(|c| !c.is_empty())
    }

    /// Removes and returns the newest checkpoint that has file changes.
    pub fn pop_checkpoint(&mut self) -> Option<Checkpoint> {
        while let Some(cp) = self.checkpoints.pop() {
            if !cp.is_empty() {
                return Some(cp);
            }
        }
        None
    }

    /// Clears history and usage; the session file (if any) is detached.
    pub fn clear(&mut self) {
        self.messages.clear();
        self.total_usage = TokenUsage::default();
        self.last_prompt_tokens = 0;
        self.checkpoints.clear();
        self.recorder = None;
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}
