//! Isolated, disposable copies of the workspace for agents that must not touch
//! the real tree, plus change detection, objective verification and safe apply.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use walkdir::{DirEntry, WalkDir};

use crate::tools::diff::unified_diff;
use crate::tools::{policy, sandbox, ToolContext, ToolError};

const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    ".venv",
    "venv",
    "__pycache__",
    "dist",
    "build",
    ".next",
    ".turbo",
    ".cache",
];
/// Dependency folders linked (not copied) into each copy so verification can run.
#[cfg(unix)]
const LINK_DIRS: &[&str] = &["node_modules", ".venv", "venv"];
const MAX_FILES: usize = 20_000;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

fn skipped(entry: &DirEntry) -> bool {
    entry.depth() > 0
        && entry.file_type().is_dir()
        && entry
            .file_name()
            .to_str()
            .is_some_and(|n| SKIP_DIRS.contains(&n))
}

fn files(root: &Path) -> io::Result<BTreeSet<PathBuf>> {
    let mut out = BTreeSet::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !skipped(e))
    {
        let entry = entry?;
        if !entry.file_type().is_file() || entry.metadata()?.len() > MAX_FILE_BYTES {
            continue;
        }
        if let Ok(rel) = entry.path().strip_prefix(root) {
            out.insert(rel.to_path_buf());
        }
        if out.len() > MAX_FILES {
            return Err(io::Error::other(format!(
                "workspace too large for isolated copies (over {MAX_FILES} files)"
            )));
        }
    }
    Ok(out)
}

pub struct WorkCopy {
    _dir: tempfile::TempDir,
    pub root: PathBuf,
}

/// Copies regular files only: symlinks could point outside the workspace.
pub fn snapshot(workspace: &Path) -> io::Result<WorkCopy> {
    let dir = tempfile::Builder::new()
        .prefix("ecliven-swarm-")
        .tempdir()?;
    let root = dir.path().canonicalize()?;
    for rel in files(workspace)? {
        let dest = root.join(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(workspace.join(&rel), dest)?;
    }
    #[cfg(unix)]
    for name in LINK_DIRS {
        let src = workspace.join(name);
        if src.is_dir() {
            let _ = std::os::unix::fs::symlink(&src, root.join(name));
        }
    }
    Ok(WorkCopy { _dir: dir, root })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDelta {
    pub rel: PathBuf,
    pub old: Option<Vec<u8>>,
    pub new: Option<Vec<u8>>,
}

pub fn changes(workspace: &Path, copy: &Path) -> io::Result<Vec<FileDelta>> {
    let original = files(workspace)?;
    let candidate = files(copy)?;
    let mut out = Vec::new();
    for rel in &candidate {
        let new = std::fs::read(copy.join(rel))?;
        let old = if original.contains(rel) {
            Some(std::fs::read(workspace.join(rel))?)
        } else {
            None
        };
        if old.as_ref() != Some(&new) {
            out.push(FileDelta {
                rel: rel.clone(),
                old,
                new: Some(new),
            });
        }
    }
    for rel in original.difference(&candidate) {
        out.push(FileDelta {
            rel: rel.clone(),
            old: Some(std::fs::read(workspace.join(rel))?),
            new: None,
        });
    }
    Ok(out)
}

pub fn render_diff(deltas: &[FileDelta]) -> String {
    let text = |b: &Option<Vec<u8>>| {
        b.as_deref()
            .map(|b| std::str::from_utf8(b).map(str::to_string))
    };
    let mut out = String::new();
    for d in deltas {
        let rel = d.rel.display();
        match (text(&d.old), text(&d.new)) {
            (Some(Err(_)), _) | (_, Some(Err(_))) => {
                out.push_str(&format!("Binary file {rel} changed\n"))
            }
            (old, new) => {
                let old = old.and_then(Result::ok).unwrap_or_default();
                let new = new.and_then(Result::ok).unwrap_or_default();
                out.push_str(&unified_diff(
                    &format!("a/{rel}"),
                    &format!("b/{rel}"),
                    &old,
                    &new,
                ));
            }
        }
    }
    out
}

pub fn changed_lines(diff: &str) -> usize {
    diff.lines()
        .filter(|l| {
            (l.starts_with('+') && !l.starts_with("+++"))
                || (l.starts_with('-') && !l.starts_with("---"))
        })
        .count()
}

pub fn is_test_path(rel: &Path) -> bool {
    rel.components().any(|c| {
        let c = c.as_os_str().to_string_lossy().to_ascii_lowercase();
        c.contains("test") || c.contains("spec")
    })
}

/// Applies deltas through the write policy; refuses everything if any file
/// changed in the real workspace since the copy was taken.
pub async fn apply(ctx: &ToolContext, deltas: &[FileDelta]) -> Result<Vec<String>, ToolError> {
    let mut planned = Vec::new();
    for d in deltas {
        let raw = d.rel.to_string_lossy().to_string();
        let path = ctx.resolve_write_path(&raw)?;
        if tokio::fs::read(&path).await.ok() != d.old {
            return Err(ToolError::Execution(format!(
                "{raw} changed in the workspace while the swarm was running; nothing was applied"
            )));
        }
        planned.push((raw, path, d));
    }
    let mut summary = Vec::new();
    for (raw, path, d) in planned {
        match &d.new {
            Some(bytes) => {
                ctx.write_file_atomic(&path, bytes).await?;
                summary.push(format!("{} {raw}", if d.old.is_some() { "M" } else { "A" }));
            }
            None => {
                tokio::fs::remove_file(&path).await?;
                summary.push(format!("D {raw}"));
            }
        }
    }
    Ok(summary)
}

pub fn detect_verify_command(root: &Path) -> Option<String> {
    let has = |f: &str| root.join(f).exists();
    let read = |f: &str| std::fs::read_to_string(root.join(f)).unwrap_or_default();
    if has("Cargo.toml") {
        Some("cargo test --quiet".into())
    } else if has("package.json") && read("package.json").contains("\"test\"") {
        Some("npm test --silent".into())
    } else if has("go.mod") {
        Some("go test ./...".into())
    } else if has("pyproject.toml") || has("pytest.ini") || has("setup.py") {
        Some("python -m pytest -q".into())
    } else if read("Makefile").lines().any(|l| l.starts_with("test:")) {
        Some("make test".into())
    } else {
        None
    }
}

#[derive(Debug, Clone)]
pub struct Verification {
    pub passed: bool,
    pub summary: String,
    pub secs: f32,
}

fn tail(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// Runs `cmd` in `root` under the same sandbox and policy as `run_command`.
pub async fn verify(
    ctx: &ToolContext,
    root: &Path,
    cmd: &str,
    timeout: Duration,
    target_dir: Option<&Path>,
) -> Verification {
    let failed = |summary: String| Verification {
        passed: false,
        summary,
        secs: 0.0,
    };
    if let Err(e) = policy::check_shell(ctx) {
        return failed(e.to_string());
    }
    let (mut command, _) = sandbox::build_command(&ctx.sandbox, root, cmd);
    command
        .current_dir(root)
        .stdin(Stdio::null())
        .kill_on_drop(true);
    if let Some(dir) = target_dir {
        command.env("CARGO_TARGET_DIR", dir);
    }
    let start = Instant::now();
    match tokio::time::timeout(timeout, command.output()).await {
        Err(_) => failed(format!("`{cmd}` timed out after {}s", timeout.as_secs())),
        Ok(Err(e)) => failed(format!("`{cmd}` could not start: {e}")),
        Ok(Ok(out)) => {
            let text = format!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            Verification {
                passed: out.status.success(),
                summary: tail(&text, 12),
                secs: start.elapsed().as_secs_f32(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_skips_build_dirs_and_detects_changes() {
        let ws = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(ws.path().join("src")).unwrap();
        std::fs::create_dir_all(ws.path().join("target/debug")).unwrap();
        std::fs::write(ws.path().join("src/a.rs"), "a\n").unwrap();
        std::fs::write(ws.path().join("src/gone.rs"), "x\n").unwrap();
        std::fs::write(ws.path().join("target/debug/big"), "bin").unwrap();
        std::fs::write(ws.path().join("Cargo.toml"), "[package]\n").unwrap();

        let copy = snapshot(ws.path()).unwrap();
        assert!(!copy.root.join("target").exists());
        assert_eq!(
            detect_verify_command(&copy.root).as_deref(),
            Some("cargo test --quiet")
        );
        assert!(changes(ws.path(), &copy.root).unwrap().is_empty());

        std::fs::write(copy.root.join("src/a.rs"), "b\n").unwrap();
        std::fs::write(copy.root.join("src/new_test.rs"), "t\n").unwrap();
        std::fs::remove_file(copy.root.join("src/gone.rs")).unwrap();
        let deltas = changes(ws.path(), &copy.root).unwrap();
        assert_eq!(deltas.len(), 3);
        let diff = render_diff(&deltas);
        assert_eq!(changed_lines(&diff), 4);
        assert!(is_test_path(Path::new("src/new_test.rs")) && !is_test_path(Path::new("src/a.rs")));
    }

    #[tokio::test]
    async fn apply_refuses_when_workspace_changed() {
        let ws = tempfile::TempDir::new().unwrap();
        std::fs::write(ws.path().join("a.txt"), "one").unwrap();
        let ctx = ToolContext::new(ws.path().to_path_buf(), 5);
        let delta = FileDelta {
            rel: "a.txt".into(),
            old: Some(b"one".to_vec()),
            new: Some(b"two".to_vec()),
        };
        std::fs::write(ws.path().join("a.txt"), "edited").unwrap();
        assert!(apply(&ctx, std::slice::from_ref(&delta)).await.is_err());
        std::fs::write(ws.path().join("a.txt"), "one").unwrap();
        assert_eq!(apply(&ctx, &[delta]).await.unwrap(), vec!["M a.txt"]);
        assert_eq!(
            std::fs::read_to_string(ws.path().join("a.txt")).unwrap(),
            "two"
        );
    }
}
