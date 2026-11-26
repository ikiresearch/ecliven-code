//! `apply_patch`: multi-file edits in a single patch
//! (`*** Begin Patch` / `*** Add File` / `*** Update File` / `*** Delete File`).

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::diff::unified_diff;
use crate::tools::{Tool, ToolContext, ToolError, ToolOutput, ToolPreview, ToolRisk};

const DESCRIPTION: &str =
    "Apply a patch that can add, update, move or delete several files at once. \
Format:\n\
*** Begin Patch\n\
*** Add File: path/new.rs\n\
+every line of the new file prefixed with +\n\
*** Update File: path/existing.rs\n\
*** Move to: path/renamed.rs   (optional)\n\
@@ optional line to locate the hunk (e.g. fn name)\n\
 context line (space prefix)\n\
-removed line\n\
+added line\n\
*** Delete File: path/old.rs\n\
*** End Patch\n\
Include ~3 lines of unchanged context around each change. Paths are relative to the workspace.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchOp {
    Add {
        path: String,
        content: String,
    },
    Delete {
        path: String,
    },
    Update {
        path: String,
        move_to: Option<String>,
        hunks: Vec<Hunk>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hunk {
    pub anchor: Option<String>,
    pub old_lines: Vec<String>,
    pub new_lines: Vec<String>,
    pub end_of_file: bool,
}

pub fn parse_patch(input: &str) -> Result<Vec<PatchOp>, ToolError> {
    let bad = |msg: String| ToolError::InvalidArgs(format!("Invalid patch: {msg}"));
    let lines: Vec<&str> = input.trim().lines().collect();
    if lines.first().map(|l| l.trim()) != Some("*** Begin Patch") {
        return Err(bad("first line must be '*** Begin Patch'".into()));
    }
    if lines.last().map(|l| l.trim()) != Some("*** End Patch") {
        return Err(bad("last line must be '*** End Patch'".into()));
    }

    let body = &lines[1..lines.len() - 1];
    let mut ops = Vec::new();
    let mut i = 0;
    while i < body.len() {
        let line = body[i];
        if let Some(path) = line.strip_prefix("*** Add File: ") {
            i += 1;
            let mut content = String::new();
            while i < body.len() && !body[i].starts_with("*** ") {
                let Some(text) = body[i].strip_prefix('+') else {
                    return Err(bad(format!(
                        "added file lines must start with '+': {}",
                        body[i]
                    )));
                };
                content.push_str(text);
                content.push('\n');
                i += 1;
            }
            ops.push(PatchOp::Add {
                path: path.trim().to_string(),
                content,
            });
        } else if let Some(path) = line.strip_prefix("*** Delete File: ") {
            ops.push(PatchOp::Delete {
                path: path.trim().to_string(),
            });
            i += 1;
        } else if let Some(path) = line.strip_prefix("*** Update File: ") {
            i += 1;
            let mut move_to = None;
            if let Some(dest) = body.get(i).and_then(|l| l.strip_prefix("*** Move to: ")) {
                move_to = Some(dest.trim().to_string());
                i += 1;
            }
            let mut hunks = Vec::new();
            let mut current: Option<Hunk> = None;
            while i < body.len() {
                let l = body[i];
                if l.trim() == "*** End of File" {
                    current.get_or_insert_with(Hunk::default).end_of_file = true;
                    i += 1;
                    continue;
                }
                if l.starts_with("*** ") {
                    break;
                }
                if let Some(anchor) = l.strip_prefix("@@") {
                    if let Some(h) = current.take() {
                        hunks.push(h);
                    }
                    let anchor = anchor.trim();
                    current = Some(Hunk {
                        anchor: (!anchor.is_empty()).then(|| anchor.to_string()),
                        ..Hunk::default()
                    });
                } else {
                    let h = current.get_or_insert_with(Hunk::default);
                    if let Some(t) = l.strip_prefix('+') {
                        h.new_lines.push(t.to_string());
                    } else if let Some(t) = l.strip_prefix('-') {
                        h.old_lines.push(t.to_string());
                    } else {
                        let t = l.strip_prefix(' ').unwrap_or(l);
                        h.old_lines.push(t.to_string());
                        h.new_lines.push(t.to_string());
                    }
                }
                i += 1;
            }
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            if hunks.is_empty() && move_to.is_none() {
                return Err(bad(format!("update of '{}' has no hunks", path.trim())));
            }
            ops.push(PatchOp::Update {
                path: path.trim().to_string(),
                move_to,
                hunks,
            });
        } else if line.trim().is_empty() {
            i += 1;
        } else {
            return Err(bad(format!("unexpected line: {line}")));
        }
    }

    if ops.is_empty() {
        return Err(bad("no file operations found".into()));
    }
    Ok(ops)
}

/// Finds `needle` in `haystack` at or after `start`, trying exact, then
/// trailing-whitespace-insensitive, then fully trimmed comparison.
fn find_lines(haystack: &[String], needle: &[String], start: usize, eof: bool) -> Option<usize> {
    if needle.is_empty() {
        return Some(if eof {
            haystack.len()
        } else {
            start.min(haystack.len())
        });
    }
    if needle.len() > haystack.len() {
        return None;
    }
    let normalizers: [fn(&str) -> &str; 3] = [|s| s, str::trim_end, str::trim];
    let last_start = haystack.len() - needle.len();
    for norm in normalizers {
        let matches_at = |pos: usize| {
            needle
                .iter()
                .zip(&haystack[pos..pos + needle.len()])
                .all(|(a, b)| norm(a) == norm(b))
        };
        if eof && matches_at(last_start) && last_start >= start {
            return Some(last_start);
        }
        if let Some(pos) = (start.min(last_start + 1)..=last_start).find(|&p| matches_at(p)) {
            return Some(pos);
        }
    }
    None
}

pub fn apply_hunks(original: &str, hunks: &[Hunk], path: &str) -> Result<String, ToolError> {
    let mut lines: Vec<String> = original.lines().map(str::to_string).collect();
    let mut cursor = 0usize;
    for hunk in hunks {
        if let Some(anchor) = &hunk.anchor {
            let found = lines[cursor..]
                .iter()
                .position(|l| l.trim() == anchor.trim())
                .or_else(|| {
                    lines[cursor..]
                        .iter()
                        .position(|l| l.contains(anchor.trim()))
                });
            if let Some(off) = found {
                cursor += off + 1;
            } else if hunk.old_lines.is_empty() {
                return Err(ToolError::Execution(format!(
                    "{path}: anchor '@@ {anchor}' not found"
                )));
            }
        }
        let pos = find_lines(&lines, &hunk.old_lines, cursor, hunk.end_of_file)
            .or_else(|| find_lines(&lines, &hunk.old_lines, 0, hunk.end_of_file))
            .ok_or_else(|| {
                ToolError::Execution(format!(
                    "{path}: could not find the lines to replace:\n{}",
                    hunk.old_lines.join("\n")
                ))
            })?;
        lines.splice(
            pos..pos + hunk.old_lines.len(),
            hunk.new_lines.iter().cloned(),
        );
        cursor = pos + hunk.new_lines.len();
    }
    let mut out = lines.join("\n");
    if !out.is_empty() || original.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// One resolved file change: `old` is None for new files, `new` is None for deletions.
struct FileChange {
    display: String,
    path: PathBuf,
    old: Option<String>,
    new: Option<String>,
}

fn plan_changes(ctx: &ToolContext, patch: &str) -> Result<Vec<FileChange>, ToolError> {
    let mut changes = Vec::new();
    for op in parse_patch(patch)? {
        match op {
            PatchOp::Add { path, content } => {
                let resolved = ctx.resolve_path(&path)?;
                let old = std::fs::read_to_string(&resolved).ok();
                changes.push(FileChange {
                    display: path,
                    path: resolved,
                    old,
                    new: Some(content),
                });
            }
            PatchOp::Delete { path } => {
                let resolved = ctx.resolve_path(&path)?;
                let old = std::fs::read_to_string(&resolved)?;
                changes.push(FileChange {
                    display: path,
                    path: resolved,
                    old: Some(old),
                    new: None,
                });
            }
            PatchOp::Update {
                path,
                move_to,
                hunks,
            } => {
                let resolved = ctx.resolve_path(&path)?;
                let old = std::fs::read_to_string(&resolved)?;
                let updated = apply_hunks(&old, &hunks, &path)?;
                match move_to {
                    Some(dest) => {
                        let dest_path = ctx.resolve_path(&dest)?;
                        let dest_old = std::fs::read_to_string(&dest_path).ok();
                        changes.push(FileChange {
                            display: path,
                            path: resolved,
                            old: Some(old),
                            new: None,
                        });
                        changes.push(FileChange {
                            display: dest,
                            path: dest_path,
                            old: dest_old,
                            new: Some(updated),
                        });
                    }
                    None => changes.push(FileChange {
                        display: path,
                        path: resolved,
                        old: Some(old),
                        new: Some(updated),
                    }),
                }
            }
        }
    }
    Ok(changes)
}

fn render_diff(changes: &[FileChange]) -> String {
    changes
        .iter()
        .map(|c| {
            unified_diff(
                &format!("a/{}", c.display),
                &format!("b/{}", c.display),
                c.old.as_deref().unwrap_or(""),
                c.new.as_deref().unwrap_or(""),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn patch_arg(args: &Value) -> Result<&str, ToolError> {
    args.get("patch")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidArgs("Missing required string field 'patch'".into()))
}

pub struct ApplyPatchTool;

#[async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "patch": { "type": "string", "description": "Full patch text from '*** Begin Patch' to '*** End Patch'" }
            },
            "required": ["patch"]
        })
    }

    fn risk_level(&self) -> ToolRisk {
        ToolRisk::FileWrite
    }

    fn touched_paths(&self, ctx: &ToolContext, args: &Value) -> Vec<PathBuf> {
        let Ok(ops) = patch_arg(args).and_then(parse_patch) else {
            return Vec::new();
        };
        ops.iter()
            .flat_map(|op| match op {
                PatchOp::Add { path, .. } | PatchOp::Delete { path } => vec![path.as_str()],
                PatchOp::Update { path, move_to, .. } => std::iter::once(path.as_str())
                    .chain(move_to.as_deref())
                    .collect(),
            })
            .filter_map(|p| ctx.resolve_path(p).ok())
            .collect()
    }

    fn preview(&self, ctx: &ToolContext, args: &Value) -> Result<ToolPreview, ToolError> {
        let changes = plan_changes(ctx, patch_arg(args)?)?;
        let files: Vec<&str> = changes.iter().map(|c| c.display.as_str()).collect();
        Ok(ToolPreview {
            summary: format!("apply_patch: {}", files.join(", ")),
            diff: Some(render_diff(&changes)),
        })
    }

    async fn execute(&self, ctx: &ToolContext, args: &Value) -> Result<ToolOutput, ToolError> {
        let changes = plan_changes(ctx, patch_arg(args)?)?;
        let mut summary = Vec::new();
        for change in &changes {
            match &change.new {
                Some(content) => {
                    if let Some(parent) = change.path.parent() {
                        tokio::fs::create_dir_all(parent).await?;
                    }
                    tokio::fs::write(&change.path, content).await?;
                    let verb = if change.old.is_some() { "M" } else { "A" };
                    summary.push(format!("{verb} {}", change.display));
                }
                None => {
                    tokio::fs::remove_file(&change.path).await?;
                    summary.push(format!("D {}", change.display));
                }
            }
        }
        Ok(ToolOutput::with_diff(
            format!("Patch applied:\n{}", summary.join("\n")),
            render_diff(&changes),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn applies_add_update_move_and_delete() {
        let tmp = TempDir::new().unwrap();
        let ctx = ToolContext::new(tmp.path().to_path_buf(), 10);
        std::fs::write(
            tmp.path().join("lib.rs"),
            "fn a() {\n    1\n}\n\nfn b() {\n    1\n}\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join("old.txt"), "bye\n").unwrap();
        std::fs::write(tmp.path().join("mv.txt"), "x\n").unwrap();

        let patch = "*** Begin Patch\n\
*** Add File: src/new.rs\n\
+pub const N: u8 = 1;\n\
*** Update File: lib.rs\n\
@@ fn b() {\n\
-    1\n\
+    2\n\
*** Update File: mv.txt\n\
*** Move to: moved.txt\n\
-x\n\
+y\n\
*** Delete File: old.txt\n\
*** End Patch";

        let tool = ApplyPatchTool;
        let args = json!({ "patch": patch });
        assert_eq!(tool.touched_paths(&ctx, &args).len(), 5);
        let out = tool.execute(&ctx, &args).await.unwrap();
        assert!(!out.is_error, "{}", out.content);

        assert_eq!(
            std::fs::read_to_string(tmp.path().join("lib.rs")).unwrap(),
            "fn a() {\n    1\n}\n\nfn b() {\n    2\n}\n"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("src/new.rs")).unwrap(),
            "pub const N: u8 = 1;\n"
        );
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("moved.txt")).unwrap(),
            "y\n"
        );
        assert!(!tmp.path().join("mv.txt").exists());
        assert!(!tmp.path().join("old.txt").exists());
    }

    #[test]
    fn tolerates_trailing_whitespace_and_reports_missing_context() {
        let hunk = Hunk {
            old_lines: vec!["let x = 1;".into()],
            new_lines: vec!["let x = 2;".into()],
            ..Hunk::default()
        };
        assert_eq!(
            apply_hunks("let x = 1;   \n", std::slice::from_ref(&hunk), "f").unwrap(),
            "let x = 2;\n"
        );
        assert!(apply_hunks("nothing here\n", &[hunk], "f").is_err());
    }

    #[test]
    fn rejects_malformed_patch_and_escaping_paths() {
        assert!(parse_patch("*** Update File: a\n-x\n+y").is_err());
        let tmp = TempDir::new().unwrap();
        let ctx = ToolContext::new(tmp.path().to_path_buf(), 10);
        let args =
            json!({ "patch": "*** Begin Patch\n*** Add File: ../evil.txt\n+x\n*** End Patch" });
        assert!(ApplyPatchTool.preview(&ctx, &args).is_err());
    }
}
