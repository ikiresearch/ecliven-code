use std::path::PathBuf;

use crate::agent::checkpoint::{Checkpoint, MAX_CHECKPOINTS};
use crate::agent::store::{LoadedSession, SessionRecorder};
use crate::ai::{Message, TokenUsage, ToolCall};

#[derive(Debug, Default, Clone)]
pub struct Session {
    messages: Vec<Message>,
    total_usage: TokenUsage,
    last_prompt_tokens: u32,
    checkpoints: Vec<Checkpoint>,
    next_checkpoint_seq: u64,
    recorder: Option<SessionRecorder>,
    pending_images: Vec<crate::ai::ImageInput>,
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

    pub fn from_loaded(loaded: LoadedSession) -> Self {
        let next_checkpoint_seq = loaded
            .checkpoints
            .iter()
            .map(|c| c.seq + 1)
            .max()
            .unwrap_or(0);
        Self {
            messages: loaded.messages,
            checkpoints: loaded.checkpoints,
            next_checkpoint_seq,
            ..Self::default()
        }
    }

    /// Latest session-file write error, if any (reported once).
    pub fn take_persist_error(&mut self) -> Option<String> {
        self.recorder.as_mut().and_then(SessionRecorder::take_error)
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
        let mut message = Message::user(content);
        message.images = std::mem::take(&mut self.pending_images);
        self.push(message);
    }

    /// Images attached to the next user message.
    pub fn attach_images(&mut self, images: Vec<crate::ai::ImageInput>) {
        self.pending_images = images;
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
        self.total_usage.cache_read_tokens = self
            .total_usage
            .cache_read_tokens
            .saturating_add(usage.cache_read_tokens);
        self.total_usage.cache_write_tokens = self
            .total_usage
            .cache_write_tokens
            .saturating_add(usage.cache_write_tokens);
        self.last_prompt_tokens = usage.input_tokens;
    }

    /// Starts the checkpoint for a new user turn (reusing an unused one).
    pub fn begin_checkpoint(&mut self, label: impl Into<String>) {
        if self.checkpoints.last().is_some_and(Checkpoint::is_empty) {
            self.checkpoints.pop();
        }
        let seq = self.next_checkpoint_seq;
        self.next_checkpoint_seq += 1;
        self.checkpoints.push(Checkpoint::new(seq, label));
        if self.checkpoints.len() > MAX_CHECKPOINTS {
            self.checkpoints.remove(0);
        }
    }

    pub fn capture_before_write(&mut self, paths: &[PathBuf]) {
        if self.checkpoints.is_empty() {
            self.begin_checkpoint("changes");
        }
        let cp = self.checkpoints.last_mut().expect("checkpoint exists");
        for p in paths {
            let (seq, label) = (cp.seq, cp.label.clone());
            if let Some(original) = cp.capture(p) {
                if let Some(rec) = self.recorder.as_mut() {
                    rec.record_checkpoint_file(seq, &label, p, original);
                }
            }
        }
    }

    pub fn checkpoints(&self) -> impl Iterator<Item = &Checkpoint> {
        self.checkpoints.iter().filter(|c| !c.is_empty())
    }

    /// Removes and returns the newest checkpoint that has file changes.
    pub fn pop_checkpoint(&mut self) -> Option<Checkpoint> {
        while let Some(cp) = self.checkpoints.pop() {
            if !cp.is_empty() {
                if let Some(rec) = self.recorder.as_mut() {
                    rec.record_checkpoint_pop(cp.seq);
                }
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
