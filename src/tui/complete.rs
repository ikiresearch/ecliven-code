//! Slash-command autocompletion for the prompt box.

pub const SLASH_COMMANDS: &[(&str, &str)] = &[
    ("/approval", "ask | auto-edit | auto-all"),
    ("/chat", "back to the chat tab"),
    ("/checkpoints", "list checkpoints"),
    ("/clear", "clear the session"),
    ("/compact", "summarize the conversation"),
    ("/connect", "connect a CLI connector"),
    ("/folder", "change workspace folder"),
    ("/goal", "set the active goal"),
    ("/help", "show help"),
    ("/history", "prompt history"),
    ("/image", "attach an image to the next message"),
    ("/index", "project index status / rebuild"),
    ("/mcp", "MCP servers"),
    ("/memory", "project and user memories"),
    ("/model", "connector/model, e.g. codex/gpt-5"),
    ("/provider", "switch connector"),
    ("/quit", "exit"),
    ("/resume", "resume a saved session"),
    ("/revert", "revert to a checkpoint"),
    ("/router", "automatic model routing"),
    ("/sandbox", "read-only | workspace-write | danger-full-access"),
    ("/saver", "token saver mode"),
    ("/scope", "limit swarm scope to a folder"),
    ("/sessions", "list saved sessions"),
    ("/setup", "run setup"),
    ("/skills", "list available skills"),
    ("/status", "tokens, cache, model"),
    ("/swarm", "code | security | competitive | collaborative | red-eclipse"),
    ("/swarm-models", "models per swarm agent"),
    ("/tasks", "task history"),
    ("/telemetry", "telemetry on/off"),
    ("/trust", "yes | deny | revoke | status"),
    ("/undo", "undo last change"),
    ("/update", "check for updates"),
    ("/workflow", "on | off"),
];

const CONNECTORS: &[&str] = &[
    "anthropic", "openai", "openrouter", "ollama", "codex", "claude", "opencode", "gemini", "agy", "cli",
];

fn arguments(command: &str) -> &'static [&'static str] {
    match command {
        "/approval" => &["ask", "auto-edit", "auto-all"],
        "/sandbox" => &["read-only", "workspace-write", "danger-full-access"],
        "/swarm" => &["code", "security", "competitive", "collaborative", "red-eclipse", "off"],
        "/trust" => &["yes", "deny", "revoke", "status"],
        "/workflow" | "/saver" | "/router" | "/telemetry" => &["on", "off"],
        "/provider" | "/connect" => CONNECTORS,
        _ => &[],
    }
}

/// Candidates for the current input as `(completion, hint)`.
pub fn suggestions(input: &str) -> Vec<(String, &'static str)> {
    if !input.starts_with('/') || input.contains('\n') {
        return Vec::new();
    }
    match input.split_once(' ') {
        None => SLASH_COMMANDS
            .iter()
            .filter(|(c, _)| c.starts_with(input))
            .map(|(c, h)| (c.to_string(), *h))
            .collect(),
        Some((cmd, arg)) if !arg.contains(' ') => {
            if cmd == "/model" {
                if arg.contains('/') {
                    return Vec::new();
                }
                return CONNECTORS
                    .iter()
                    .filter(|c| c.starts_with(arg))
                    .map(|c| (format!("{cmd} {c}/"), "connector"))
                    .collect();
            }
            arguments(cmd)
                .iter()
                .filter(|a| a.starts_with(arg))
                .map(|a| (format!("{cmd} {a}"), ""))
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Tab result: the single match (plus a space unless it ends in `/`), or the
/// longest common prefix of several matches.
pub fn complete(input: &str) -> Option<String> {
    let found = suggestions(input);
    let first = found.first()?.0.clone();
    if found.len() == 1 {
        return Some(if first.ends_with('/') { first } else { format!("{first} ") });
    }
    let mut prefix = first;
    for (s, _) in &found[1..] {
        while !s.starts_with(&prefix) {
            prefix.pop();
        }
    }
    (prefix.len() > input.len()).then_some(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completes_commands_arguments_and_connectors() {
        assert_eq!(complete("/sk").as_deref(), Some("/skills "));
        assert_eq!(complete("/swarm-").as_deref(), Some("/swarm-models "));
        assert_eq!(complete("/sw").as_deref(), Some("/swarm"));
        assert_eq!(complete("/swarm red").as_deref(), Some("/swarm red-eclipse "));
        assert_eq!(complete("/model cod").as_deref(), Some("/model codex/"));
        assert_eq!(complete("/model codex/x"), None);
        assert!(complete("hello").is_none());
        assert!(suggestions("/").len() == SLASH_COMMANDS.len());
    }
}
