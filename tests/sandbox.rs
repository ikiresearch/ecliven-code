#![cfg(target_os = "linux")]

use std::path::PathBuf;

use ecliven_code::tools::sandbox::{SandboxMode, SandboxPolicy};
use ecliven_code::tools::shell::RunCommandTool;
use ecliven_code::tools::{Tool, ToolContext};
use serde_json::json;
use tempfile::TempDir;

fn landlock_enforced() -> bool {
    std::fs::read_to_string("/sys/kernel/security/lsm")
        .map(|lsm| lsm.contains("landlock"))
        .unwrap_or(false)
}

fn ctx(workspace: &TempDir, mode: SandboxMode) -> ToolContext {
    let mut policy = SandboxPolicy::new(mode, true, Vec::new());
    policy.helper_exe = Some(PathBuf::from(env!("CARGO_BIN_EXE_ecliven")));
    ToolContext::new(workspace.path().to_path_buf(), 20).with_sandbox(policy)
}

#[tokio::test]
async fn workspace_write_allows_workspace_and_blocks_outside() {
    if !landlock_enforced() {
        eprintln!("skipping: Landlock not enabled on this kernel");
        return;
    }
    let ws = TempDir::new().unwrap();
    let outside = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let target = outside.path().join("escape.txt");
    let ctx = ctx(&ws, SandboxMode::WorkspaceWrite);

    let ok = RunCommandTool
        .execute(
            &ctx,
            &json!({ "command": "echo hi > inside.txt && cat inside.txt" }),
        )
        .await
        .unwrap();
    assert!(ok.content.contains("Exit code: 0"), "{}", ok.content);
    assert!(ws.path().join("inside.txt").exists());

    let cmd = format!("echo no > '{}'", target.display());
    let denied = RunCommandTool
        .execute(&ctx, &json!({ "command": cmd }))
        .await
        .unwrap();
    assert!(
        !target.exists(),
        "write outside the workspace must be blocked: {}",
        denied.content
    );
}

#[tokio::test]
async fn read_only_blocks_workspace_writes_but_allows_reads() {
    if !landlock_enforced() {
        eprintln!("skipping: Landlock not enabled on this kernel");
        return;
    }
    // Temp dirs stay writable in read-only mode, so keep the workspace elsewhere.
    let ws = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    std::fs::write(ws.path().join("a.txt"), "data").unwrap();
    let ctx = ctx(&ws, SandboxMode::ReadOnly);

    let read = RunCommandTool
        .execute(&ctx, &json!({ "command": "cat a.txt" }))
        .await
        .unwrap();
    assert!(read.content.contains("data"), "{}", read.content);

    RunCommandTool
        .execute(&ctx, &json!({ "command": "echo x > b.txt" }))
        .await
        .unwrap();
    assert!(!ws.path().join("b.txt").exists());
}
