//! Structured output of external coding CLIs: the final answer plus every tool
//! call they ran (commands, file changes, MCP calls), parsed from their JSON event streams.

use std::collections::HashMap;

use serde_json::Value;

use crate::ai::{ExternalToolCall, TokenUsage};

const MAX_TOOL_OUTPUT: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliFormat {
    Text,
    /// `codex exec --json` JSONL events.
    CodexJson,
    /// `claude -p --output-format stream-json --verbose` events.
    ClaudeStream,
}

#[derive(Debug, Default)]
pub struct CliTranscript {
    pub text: String,
    pub tools: Vec<ExternalToolCall>,
    pub usage: Option<TokenUsage>,
    pub error: Option<String>,
    pending: HashMap<String, (String, String)>,
}

fn clip(s: &str) -> String {
    let t = crate::tools::truncate_str(s.trim(), MAX_TOOL_OUTPUT);
    if t.len() < s.trim().len() {
        format!("{t}…")
    } else {
        t.to_string()
    }
}

fn usage_of(v: &Value) -> Option<TokenUsage> {
    let input = v.get("input_tokens")?.as_u64()?;
    let output = v.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
    Some(TokenUsage {
        input_tokens: input as u32,
        output_tokens: output as u32,
    })
}

impl CliTranscript {
    fn push_text(&mut self, text: &str) -> Option<String> {
        if text.trim().is_empty() {
            return None;
        }
        let delta = if self.text.is_empty() {
            text.to_string()
        } else {
            format!("\n\n{text}")
        };
        self.text.push_str(&delta);
        Some(delta)
    }

    fn tool(&mut self, name: String, input: String, output: String, ok: bool) {
        self.tools.push(ExternalToolCall {
            name,
            input: clip(&input),
            output: clip(&output),
            ok,
        });
    }

    /// Feeds one stdout line; returns text to stream to the user.
    pub fn feed(&mut self, format: CliFormat, line: &str) -> Option<String> {
        if format == CliFormat::Text {
            self.text.push_str(line);
            return Some(line.to_string());
        }
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else {
            return None;
        };
        match format {
            CliFormat::CodexJson => self.feed_codex(&v),
            CliFormat::ClaudeStream => self.feed_claude(&v),
            CliFormat::Text => None,
        }
    }

    fn feed_codex(&mut self, v: &Value) -> Option<String> {
        let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        match v.get("type").and_then(Value::as_str)? {
            "item.completed" => {
                let item = v.get("item")?;
                let status = s(item, "status");
                match item.get("type").and_then(Value::as_str)? {
                    "agent_message" => return self.push_text(&s(item, "text")),
                    "command_execution" => {
                        let ok = item.get("exit_code").and_then(Value::as_i64) == Some(0);
                        self.tool("shell".into(), s(item, "command"), s(item, "aggregated_output"), ok);
                    }
                    "file_change" => {
                        let changes: Vec<String> = item
                            .get("changes")
                            .and_then(Value::as_array)
                            .map(|a| a.iter().map(|c| format!("{} {}", s(c, "kind"), s(c, "path"))).collect())
                            .unwrap_or_default();
                        self.tool("file_change".into(), changes.join(", "), status.clone(), status == "completed");
                    }
                    "mcp_tool_call" => {
                        let output = match (item.get("result"), item.get("error")) {
                            (_, Some(e)) if !e.is_null() => s(e, "message"),
                            (Some(r), _) if !r.is_null() => r.to_string(),
                            _ => status.clone(),
                        };
                        let name = format!("mcp:{}/{}", s(item, "server"), s(item, "tool"));
                        let args = item.get("arguments").map(Value::to_string).unwrap_or_default();
                        self.tool(name, args, output, status == "completed");
                    }
                    "web_search" => self.tool("web_search".into(), s(item, "query"), String::new(), true),
                    "error" => self.error = Some(s(item, "message")),
                    _ => {}
                }
            }
            "turn.completed" => self.usage = v.get("usage").and_then(usage_of),
            "turn.failed" => self.error = v.get("error").map(|e| s(e, "message")),
            "error" => self.error = Some(s(v, "message")),
            _ => {}
        }
        None
    }

    fn feed_claude(&mut self, v: &Value) -> Option<String> {
        let content = || {
            v.get("message")
                .and_then(|m| m.get("content"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };
        match v.get("type").and_then(Value::as_str)? {
            "assistant" => {
                let mut delta = String::new();
                for block in content() {
                    match block.get("type").and_then(Value::as_str) {
                        Some("text") => {
                            if let Some(d) = self.push_text(block.get("text").and_then(Value::as_str).unwrap_or("")) {
                                delta.push_str(&d);
                            }
                        }
                        Some("tool_use") => {
                            let id = block.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                            let name = block.get("name").and_then(Value::as_str).unwrap_or("tool").to_string();
                            let input = block.get("input").map(Value::to_string).unwrap_or_default();
                            self.pending.insert(id, (name, input));
                        }
                        _ => {}
                    }
                }
                return (!delta.is_empty()).then_some(delta);
            }
            "user" => {
                for block in content() {
                    if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                        continue;
                    }
                    let id = block.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
                    let output = match block.get("content") {
                        Some(Value::String(s)) => s.clone(),
                        Some(Value::Array(parts)) => parts
                            .iter()
                            .filter_map(|p| p.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => String::new(),
                    };
                    let ok = !block.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                    let (name, input) = self.pending.remove(id).unwrap_or_else(|| ("tool".into(), String::new()));
                    self.tool(name, input, output, ok);
                }
            }
            "result" => {
                let result = v.get("result").and_then(Value::as_str).unwrap_or("");
                if v.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
                    self.error = Some(result.to_string());
                } else if self.text.is_empty() {
                    return self.push_text(result);
                }
                self.usage = v.get("usage").and_then(usage_of);
            }
            _ => {}
        }
        None
    }

    pub fn finish(&mut self) {
        let pending: Vec<_> = self.pending.drain().collect();
        for (_, (name, input)) in pending {
            self.tool(name, input, "(no result reported)".into(), false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(format: CliFormat, lines: &[&str]) -> CliTranscript {
        let mut t = CliTranscript::default();
        for l in lines {
            t.feed(format, l);
        }
        t.finish();
        t
    }

    #[test]
    fn parses_codex_events() {
        let t = run(CliFormat::CodexJson, &[
            r#"{"type":"thread.started","thread_id":"t"}"#,
            r#"{"type":"item.completed","item":{"id":"1","type":"command_execution","command":"cargo test","aggregated_output":"ok","exit_code":0,"status":"completed"}}"#,
            r#"{"type":"item.completed","item":{"id":"2","type":"file_change","changes":[{"path":"src/a.rs","kind":"update"}],"status":"completed"}}"#,
            r#"{"type":"item.completed","item":{"id":"3","type":"mcp_tool_call","server":"db","tool":"query","arguments":{"q":1},"result":null,"error":{"message":"denied"},"status":"failed"}}"#,
            r#"{"type":"item.completed","item":{"id":"4","type":"agent_message","text":"Done."}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":10,"cached_input_tokens":0,"output_tokens":3}}"#,
            "not json",
        ]);
        assert_eq!(t.text, "Done.");
        assert_eq!(t.tools.len(), 3);
        assert_eq!((t.tools[0].name.as_str(), t.tools[0].input.as_str(), t.tools[0].ok), ("shell", "cargo test", true));
        assert_eq!(t.tools[1].input, "update src/a.rs");
        assert_eq!((t.tools[2].name.as_str(), t.tools[2].output.as_str(), t.tools[2].ok), ("mcp:db/query", "denied", false));
        assert_eq!(t.usage.unwrap().input_tokens, 10);
    }

    #[test]
    fn parses_claude_stream() {
        let t = run(CliFormat::ClaudeStream, &[
            r#"{"type":"system","subtype":"init"}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Checking."},{"type":"tool_use","id":"u1","name":"Bash","input":{"command":"ls"}},{"type":"tool_use","id":"u2","name":"Read","input":{}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"u1","content":[{"type":"text","text":"a.rs"}],"is_error":false}]}}"#,
            r#"{"type":"result","subtype":"success","result":"Checking.","usage":{"input_tokens":7,"output_tokens":2}}"#,
        ]);
        assert_eq!(t.text, "Checking.");
        assert_eq!(t.tools.len(), 2);
        assert_eq!((t.tools[0].name.as_str(), t.tools[0].output.as_str()), ("Bash", "a.rs"));
        assert!(t.tools[0].input.contains("ls"));
        assert!(!t.tools[1].ok);
        assert_eq!(t.usage.unwrap().output_tokens, 2);
    }
}
