use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::{Tool, ToolContext, ToolError, ToolOutput, ToolPreview, ToolRisk};

const MAX_OUTPUT_CHARS: usize = 12_000;

pub struct RunCommandTool;

#[async_trait]
impl Tool for RunCommandTool {
    fn name(&self) -> &str {
        "run_command"
    }

    fn description(&self) -> &str {
        "Run a shell command in the workspace root (e.g. cargo test, git status). Captures stdout, stderr, and exit code."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute" }
            },
            "required": ["command"]
        })
    }

    fn risk_level(&self) -> ToolRisk {
        ToolRisk::ShellExec
    }

    fn preview(&self, _ctx: &ToolContext, args: &Value) -> Result<ToolPreview, ToolError> {
        let cmd = args
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("Missing required string 'command'".into()))?;
        Ok(ToolPreview {
            summary: format!("run_command: $ {cmd}"),
            diff: None,
        })
    }

    async fn execute(&self, ctx: &ToolContext, args: &Value) -> Result<ToolOutput, ToolError> {
        let cmd_str = args
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("Missing required string 'command'".into()))?;

        let (mut command, sandbox_note) =
            crate::tools::sandbox::build_command(&ctx.sandbox, &ctx.workspace_root, cmd_str);
        command.current_dir(&ctx.workspace_root).kill_on_drop(true);

        let timeout_dur = Duration::from_secs(ctx.shell_timeout_secs);
        let output = tokio::time::timeout(timeout_dur, command.output())
            .await
            .map_err(|_| {
                ToolError::Execution(format!(
                    "Command timed out after {} seconds",
                    ctx.shell_timeout_secs
                ))
            })??;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let code = output.status.code().unwrap_or(-1);

        let mut combined = format!("Exit code: {code}\n");
        if let Some(note) = sandbox_note {
            combined.push_str(&note);
            combined.push('\n');
        }
        if !stdout.trim().is_empty() {
            combined.push_str("\n--- stdout ---\n");
            combined.push_str(&stdout);
        }
        if !stderr.trim().is_empty() {
            combined.push_str("\n--- stderr ---\n");
            combined.push_str(&stderr);
        }

        if combined.len() > MAX_OUTPUT_CHARS {
            let keep = crate::tools::truncate_str(&combined, MAX_OUTPUT_CHARS).len();
            combined.truncate(keep);
            combined.push_str("\n... [output truncated]");
        }

        if output.status.success() {
            Ok(ToolOutput::ok(combined))
        } else {
            Ok(ToolOutput::err(combined))
        }
    }
}
