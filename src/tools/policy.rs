//! Central security policy applied to every tool call, independent of how each
//! tool is implemented: sandbox mode, platform support and approval mode.

use crate::config::ApprovalMode;
use crate::tools::sandbox::SandboxMode;
use crate::tools::{ToolContext, ToolError, ToolRisk};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// Ask the user; `note` explains a forced prompt.
    Ask { note: Option<String> },
    Deny(String),
}

/// Whether `run_command` can be isolated by the OS on this platform.
pub fn os_sandbox_available() -> bool {
    cfg!(any(target_os = "linux", target_os = "macos"))
}

fn is_mcp(tool_name: &str) -> bool {
    tool_name.starts_with("mcp__")
}

pub fn evaluate(tool_name: &str, risk: ToolRisk, ctx: &ToolContext, approval: ApprovalMode) -> Decision {
    let mode = ctx.sandbox.mode;
    match risk {
        ToolRisk::FileWrite if mode == SandboxMode::ReadOnly => Decision::Deny(
            "read-only sandbox: file changes are disabled (switch with /sandbox workspace-write)".into(),
        ),
        ToolRisk::ShellExec if is_mcp(tool_name) && mode == SandboxMode::ReadOnly => Decision::Deny(
            "read-only sandbox: untrusted MCP tools are disabled because they run outside the sandbox".into(),
        ),
        ToolRisk::ShellExec
            if !is_mcp(tool_name) && ctx.sandbox.is_restricted() && !os_sandbox_available() =>
        {
            if mode == SandboxMode::ReadOnly {
                Decision::Deny("read-only sandbox is not available on this OS: commands are blocked".into())
            } else {
                Decision::Ask {
                    note: Some(format!(
                        "no OS sandbox on this platform: the command would run unsandboxed ({} requested)",
                        mode.as_str()
                    )),
                }
            }
        }
        _ if risk.requires_approval(approval) => Decision::Ask { note: None },
        _ => Decision::Allow,
    }
}

/// Defense in depth for write tools that may be called outside the agent loop.
pub fn check_file_write(ctx: &ToolContext) -> Result<(), ToolError> {
    match evaluate("file_write", ToolRisk::FileWrite, ctx, ApprovalMode::AutoAll) {
        Decision::Deny(reason) => Err(ToolError::PolicyDenied(reason)),
        _ => Ok(()),
    }
}

/// Blocks shell commands that would silently run without the requested isolation.
pub fn check_shell(ctx: &ToolContext) -> Result<(), ToolError> {
    match evaluate("run_command", ToolRisk::ShellExec, ctx, ApprovalMode::AutoAll) {
        Decision::Deny(reason) => Err(ToolError::PolicyDenied(reason)),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::sandbox::SandboxPolicy;

    fn ctx(mode: SandboxMode) -> ToolContext {
        ToolContext::new(std::env::temp_dir(), 5).with_sandbox(SandboxPolicy::new(mode, true, Vec::new()))
    }

    #[test]
    fn read_only_blocks_every_writer() {
        let ro = ctx(SandboxMode::ReadOnly);
        for (name, risk) in [("write_file", ToolRisk::FileWrite), ("apply_patch", ToolRisk::FileWrite), ("mcp__x__y", ToolRisk::ShellExec)] {
            assert!(matches!(evaluate(name, risk, &ro, ApprovalMode::AutoAll), Decision::Deny(_)), "{name}");
        }
        assert_eq!(evaluate("read_file", ToolRisk::ReadOnly, &ro, ApprovalMode::Ask), Decision::Allow);
        assert!(check_file_write(&ro).is_err());
        assert!(check_file_write(&ctx(SandboxMode::WorkspaceWrite)).is_ok());
    }

    #[test]
    fn approval_mode_still_applies() {
        let ww = ctx(SandboxMode::WorkspaceWrite);
        assert_eq!(
            evaluate("write_file", ToolRisk::FileWrite, &ww, ApprovalMode::Ask),
            Decision::Ask { note: None }
        );
        assert_eq!(evaluate("write_file", ToolRisk::FileWrite, &ww, ApprovalMode::AutoAll), Decision::Allow);
        let shell = evaluate("run_command", ToolRisk::ShellExec, &ww, ApprovalMode::AutoAll);
        if os_sandbox_available() {
            assert_eq!(shell, Decision::Allow);
        } else {
            assert!(matches!(shell, Decision::Ask { note: Some(_) }));
        }
        let full = ctx(SandboxMode::DangerFullAccess);
        assert_eq!(evaluate("run_command", ToolRisk::ShellExec, &full, ApprovalMode::AutoAll), Decision::Allow);
    }
}
