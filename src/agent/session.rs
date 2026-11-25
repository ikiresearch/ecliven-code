use crate::ai::{Message, TokenUsage, ToolCall};

#[derive(Debug, Default, Clone)]
pub struct Session {
    messages: Vec<Message>,
    total_usage: TokenUsage,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    pub fn total_usage(&self) -> &TokenUsage {
        &self.total_usage
    }

    pub fn push_user(&mut self, content: impl Into<String>) {
        self.messages.push(Message::user(content));
    }

    pub fn push_assistant(&mut self, content: impl Into<String>, tool_calls: Vec<ToolCall>) {
        self.messages.push(Message::assistant(content, tool_calls));
    }

    pub fn push_tool_result(
        &mut self,
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        output: impl Into<String>,
    ) {
        self.messages
            .push(Message::tool_result(tool_call_id, tool_name, output));
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
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.total_usage = TokenUsage::default();
    }

    pub fn len(&self) -> usize {
        self.messages.len()
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}
