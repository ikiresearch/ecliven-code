use serde_json::json;
use tempfile::TempDir;

use ecliven_code::tools::sandbox::{SandboxMode, SandboxPolicy};
use ecliven_code::tools::{ToolContext, ToolError, ToolRegistry};

async fn run(ctx: &ToolContext, tool: &str, args: serde_json::Value) -> Result<String, ToolError> {
    let reg = ToolRegistry::default_registry();
    reg.get(tool).unwrap().execute(ctx, &args).await.map(|o| o.content)
}

#[cfg(unix)]
#[tokio::test]
async fn new_files_cannot_escape_through_symlinks() {
    let ws = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    std::os::unix::fs::symlink(outside.path(), ws.path().join("linkdir")).unwrap();
    std::os::unix::fs::symlink(outside.path().join("missing.txt"), ws.path().join("dangling")).unwrap();
    let ctx = ToolContext::new(ws.path().to_path_buf(), 5);

    for path in ["linkdir/new.txt", "linkdir/sub/new.txt", "dangling"] {
        let err = run(&ctx, "write_file", json!({ "path": path, "content": "x" })).await;
        assert!(err.is_err(), "{path} should be rejected");
    }
    let patch = "*** Begin Patch\n*** Add File: linkdir/p.txt\n+x\n*** End Patch";
    assert!(run(&ctx, "apply_patch", json!({ "patch": patch })).await.is_err());
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);

    std::fs::create_dir(ws.path().join("real")).unwrap();
    std::os::unix::fs::symlink(ws.path().join("real"), ws.path().join("inner")).unwrap();
    run(&ctx, "write_file", json!({ "path": "inner/ok.txt", "content": "ok" })).await.unwrap();
    assert_eq!(std::fs::read_to_string(ws.path().join("real/ok.txt")).unwrap(), "ok");
}

#[tokio::test]
async fn git_internals_and_read_only_mode_are_protected() {
    let ws = TempDir::new().unwrap();
    std::fs::create_dir_all(ws.path().join(".git/hooks")).unwrap();
    std::fs::write(ws.path().join("a.txt"), "one\n").unwrap();
    let ctx = ToolContext::new(ws.path().to_path_buf(), 5);
    let err = run(&ctx, "write_file", json!({ "path": ".git/hooks/pre-commit", "content": "x" })).await;
    assert!(matches!(err, Err(ToolError::PolicyDenied(_))));

    let ro = ctx.clone().with_sandbox(SandboxPolicy::new(SandboxMode::ReadOnly, true, Vec::new()));
    for (tool, args) in [
        ("write_file", json!({ "path": "b.txt", "content": "x" })),
        ("edit_file", json!({ "path": "a.txt", "old_string": "one", "new_string": "two" })),
        ("apply_patch", json!({ "patch": "*** Begin Patch\n*** Delete File: a.txt\n*** End Patch" })),
    ] {
        assert!(matches!(run(&ro, tool, args).await, Err(ToolError::PolicyDenied(_))), "{tool}");
    }
    assert_eq!(std::fs::read_to_string(ws.path().join("a.txt")).unwrap(), "one\n");
    assert!(!ws.path().join("b.txt").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn atomic_writes_keep_permissions_and_leave_no_temp_files() {
    use std::os::unix::fs::PermissionsExt;
    let ws = TempDir::new().unwrap();
    let script = ws.path().join("run.sh");
    std::fs::write(&script, "echo 1\n").unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let ctx = ToolContext::new(ws.path().to_path_buf(), 5);
    run(&ctx, "edit_file", json!({ "path": "run.sh", "old_string": "1", "new_string": "2" })).await.unwrap();
    assert_eq!(std::fs::read_to_string(&script).unwrap(), "echo 2\n");
    assert_eq!(std::fs::metadata(&script).unwrap().permissions().mode() & 0o777, 0o755);
    assert_eq!(std::fs::read_dir(ws.path()).unwrap().count(), 1);
}
