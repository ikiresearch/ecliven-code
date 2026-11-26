use std::path::{Path, PathBuf};
use std::process::Stdio;

use async_trait::async_trait;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::mpsc;

use crate::ai::{
    AiError, CompletionRequest, CompletionResponse, LlmProvider, Message, Role, TokenUsage,
};
use crate::config::ProviderKind;

#[derive(Debug, Clone)]
pub struct DetectedCli {
    pub kind: ProviderKind,
    pub binary: &'static str,
    pub path: Option<PathBuf>,
}

pub fn detect_installed_clis() -> Vec<DetectedCli> {
    ProviderKind::EXTERNAL_CLIS
        .iter()
        .copied()
        .map(|kind| {
            let binary = kind.default_base_url();
            let path = find_executable_in_path(binary);
            DetectedCli { kind, binary, path }
        })
        .collect()
}

pub fn find_executable_in_path(binary: &str) -> Option<PathBuf> {
    if binary.trim().is_empty() {
        return None;
    }
    let path_var = std::env::var_os("PATH")?;
    let candidates: Vec<String> = if cfg!(windows) {
        let lower = binary.to_ascii_lowercase();
        if lower.ends_with(".exe") || lower.ends_with(".cmd") || lower.ends_with(".bat") {
            vec![binary.to_string()]
        } else {
            vec![
                format!("{binary}.exe"),
                format!("{binary}.cmd"),
                format!("{binary}.bat"),
                format!("{binary}.ps1"),
                binary.to_string(),
            ]
        }
    } else {
        vec![binary.to_string()]
    };

    for dir in std::env::split_paths(&path_var) {
        for cand in &candidates {
            let full = dir.join(cand);
            if full.is_file() {
                return Some(full);
            }
        }
    }
    None
}

pub struct ExternalCliProvider {
    kind: ProviderKind,
    binary_or_cmd: String,
    model: String,
    workspace_dir: PathBuf,
}

impl ExternalCliProvider {
    pub fn new(
        kind: ProviderKind,
        binary_or_cmd: String,
        model: String,
        workspace_dir: PathBuf,
    ) -> Self {
        Self {
            kind,
            binary_or_cmd,
            model,
            workspace_dir,
        }
    }

    fn build_command_and_args(&self, prompt: &str) -> Result<(String, Vec<String>, bool), AiError> {
        let custom_model = (!self.model.trim().is_empty() && self.model != "default")
            .then_some(self.model.as_str());

        match self.kind {
            ProviderKind::CodexCli => {
                let mut args = vec![
                    "exec".to_string(),
                    "--skip-git-repo-check".to_string(),
                    "--color".to_string(),
                    "never".to_string(),
                    "-C".to_string(),
                    self.workspace_dir.display().to_string(),
                ];
                if let Some(m) = custom_model {
                    args.push("-m".to_string());
                    args.push(m.to_string());
                }
                args.push("-".to_string());
                Ok((self.binary_or_cmd.clone(), args, true))
            }
            ProviderKind::AgyCli => {
                let mut args = vec!["--output-format".to_string(), "text".to_string()];
                if let Some(m) = custom_model {
                    args.push("--model".to_string());
                    args.push(m.to_string());
                }
                args.push("--print".to_string());
                args.push(prompt.to_string());
                Ok((self.binary_or_cmd.clone(), args, false))
            }
            ProviderKind::ClaudeCli => {
                let mut args = vec![
                    "--print".to_string(),
                    "--output-format".to_string(),
                    "text".to_string(),
                ];
                if let Some(m) = custom_model {
                    args.push("--model".to_string());
                    args.push(m.to_string());
                }
                Ok((self.binary_or_cmd.clone(), args, true))
            }
            ProviderKind::OpenCodeCli => {
                let mut args = vec!["run".to_string()];
                if let Some(m) = custom_model {
                    args.push("--model".to_string());
                    args.push(m.to_string());
                }
                args.push(prompt.to_string());
                Ok((self.binary_or_cmd.clone(), args, false))
            }
            ProviderKind::GeminiCli => {
                let mut args = Vec::new();
                if let Some(m) = custom_model {
                    args.push("-m".to_string());
                    args.push(m.to_string());
                }
                args.push("-p".to_string());
                args.push(String::new());
                Ok((self.binary_or_cmd.clone(), args, true))
            }
            ProviderKind::CustomCli => {
                let mut tokens = self.binary_or_cmd.split_whitespace();
                let bin = tokens.next().ok_or_else(|| {
                    AiError::Parse(
                        "Custom CLI command is empty. Use '/connect custom <command> [args...]' or set ECLIVEN_CLI_CMD."
                            .to_string(),
                    )
                })?;
                let args: Vec<String> = tokens.map(ToString::to_string).collect();
                Ok((bin.to_string(), args, true))
            }
            _ => Err(AiError::Parse(format!(
                "Provider '{}' is not a CLI backend",
                self.kind
            ))),
        }
    }
}

fn spawn_resolved(program: &str, args: &[String], cwd: &Path) -> Result<Command, AiError> {
    if cfg!(windows) {
        if let Some(found) = find_executable_in_path(program) {
            let ext = found
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext == "ps1" {
                let mut c = Command::new("powershell");
                c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
                c.arg(&found);
                c.args(args);
                c.current_dir(cwd);
                return Ok(c);
            }
            if ext == "cmd" || ext == "bat" {
                let mut c = Command::new("cmd");
                c.arg("/C").arg(&found).args(args);
                c.current_dir(cwd);
                return Ok(c);
            }
            let mut c = Command::new(found);
            c.args(args);
            c.current_dir(cwd);
            return Ok(c);
        }
    }

    let mut c = Command::new(program);
    c.args(args);
    c.current_dir(cwd);
    Ok(c)
}

fn format_cli_prompt(request: &CompletionRequest) -> String {
    let mut out = String::new();
    if !request.system_prompt.trim().is_empty() {
        out.push_str("System instructions:\n");
        out.push_str(request.system_prompt.trim());
        out.push_str("\n\n");
    }

    let history = &request.messages;
    if history.len() == 1 && history[0].role == Role::User {
        out.push_str(&history[0].content);
        return out;
    }

    out.push_str("Conversation:\n");
    for msg in history {
        format_message_line(&mut out, msg);
    }
    out
}

fn format_message_line(out: &mut String, msg: &Message) {
    let label = match msg.role {
        Role::System => "System",
        Role::User => "User",
        Role::Assistant => "Assistant",
        Role::Tool => "Tool",
    };
    out.push_str(label);
    out.push_str(":\n");
    out.push_str(&msg.content);
    out.push_str("\n\n");
}

#[async_trait]
impl LlmProvider for ExternalCliProvider {
    fn name(&self) -> &'static str {
        self.kind.as_str()
    }

    fn model(&self) -> &str {
        &self.model
    }

    async fn complete(
        &self,
        request: &CompletionRequest,
        delta_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<CompletionResponse, AiError> {
        let prompt = format_cli_prompt(request);
        let (program, args, pipe_stdin) = self.build_command_and_args(&prompt)?;

        let mut cmd = spawn_resolved(&program, &args, &self.workspace_dir)?;
        cmd.stdin(if pipe_stdin {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|err| {
            AiError::Parse(format!(
                "Failed to start external CLI '{program}': {err}. Is it installed and on your PATH?"
            ))
        })?;

        if pipe_stdin {
            if let Some(mut stdin) = child.stdin.take() {
                let bytes = prompt.as_bytes().to_vec();
                tokio::spawn(async move {
                    let _ = stdin.write_all(&bytes).await;
                    let _ = stdin.shutdown().await;
                });
            }
        }

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| AiError::Parse("Failed to capture CLI stdout".into()))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| AiError::Parse("Failed to capture CLI stderr".into()))?;

        let stderr_handle = tokio::spawn(async move {
            let mut err_buf = Vec::new();
            let _ = stderr.read_to_end(&mut err_buf).await;
            String::from_utf8_lossy(&err_buf).to_string()
        });

        let mut full_output = String::new();
        let mut chunk = [0u8; 1024];
        loop {
            match stdout.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&chunk[..n]).to_string();
                    full_output.push_str(&text);
                    if let Some(ref tx) = delta_tx {
                        let _ = tx.send(text);
                    }
                }
                Err(err) => {
                    return Err(AiError::Parse(format!(
                        "Error reading stdout from '{program}': {err}"
                    )));
                }
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| AiError::Parse(format!("Failed waiting for '{program}': {e}")))?;
        let stderr_text = stderr_handle.await.unwrap_or_default();

        if !status.success() && full_output.trim().is_empty() {
            return Err(AiError::CliFailed {
                cli: program,
                code: status.code().unwrap_or(1),
                output: if stderr_text.trim().is_empty() {
                    format!("exited with status {status}")
                } else {
                    stderr_text.trim().to_string()
                },
            });
        }

        let input_tokens = (prompt.chars().count() / 4) as u32;
        let output_tokens = (full_output.chars().count() / 4) as u32;

        Ok(CompletionResponse {
            content: full_output.trim_end().to_string(),
            tool_calls: Vec::new(),
            usage: Some(TokenUsage {
                input_tokens,
                output_tokens,
            }),
            stop_reason: Some("end_turn".to_string()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_codex_and_agy_cli_arguments() {
        let codex = ExternalCliProvider::new(
            ProviderKind::CodexCli,
            "codex".into(),
            "o3".into(),
            PathBuf::from("."),
        );
        let (bin, args, stdin) = codex.build_command_and_args("hi").unwrap();
        assert_eq!(bin, "codex");
        assert!(stdin);
        assert!(args.contains(&"exec".to_string()));
        assert!(args.contains(&"--skip-git-repo-check".to_string()));
        assert!(args.contains(&"o3".to_string()));

        let agy = ExternalCliProvider::new(
            ProviderKind::AgyCli,
            "agy".into(),
            "default".into(),
            PathBuf::from("."),
        );
        let (bin2, args2, stdin2) = agy.build_command_and_args("hi").unwrap();
        assert_eq!(bin2, "agy");
        assert!(!stdin2);
        assert_eq!(args2, vec!["--output-format", "text", "--print", "hi"]);
    }

    #[test]
    fn builds_custom_cli_command_and_flags() {
        let custom = ExternalCliProvider::new(
            ProviderKind::CustomCli,
            "my-agent --fast --plain".into(),
            "default".into(),
            PathBuf::from("."),
        );
        let (bin, args, stdin) = custom.build_command_and_args("test").unwrap();
        assert_eq!(bin, "my-agent");
        assert_eq!(args, vec!["--fast", "--plain"]);
        assert!(stdin);
    }
}
