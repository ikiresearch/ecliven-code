//! Loads project instructions from AGENTS.md files: a global file, then one
//! file per directory from the repository root down to the workspace, with
//! `AGENTS.override.md` taking precedence.

use std::path::{Path, PathBuf};

pub const MAX_PROJECT_DOC_BYTES: usize = 32 * 1024;
const CANDIDATES: [&str; 3] = ["AGENTS.override.md", "AGENTS.md", "CLAUDE.md"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstructionFile {
    pub path: PathBuf,
    pub content: String,
}

pub fn global_instructions_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("ecliven-code").join("AGENTS.md"))
}

pub fn load_project_instructions(workspace: &Path) -> Vec<InstructionFile> {
    load_instructions_with(
        global_instructions_path().as_deref(),
        workspace,
        MAX_PROJECT_DOC_BYTES,
    )
}

pub fn load_instructions_with(
    global: Option<&Path>,
    workspace: &Path,
    budget: usize,
) -> Vec<InstructionFile> {
    let mut paths: Vec<PathBuf> = global
        .filter(|p| p.is_file())
        .map(Path::to_path_buf)
        .into_iter()
        .collect();
    for dir in dirs_from_repo_root(workspace) {
        if let Some(found) = CANDIDATES.iter().map(|n| dir.join(n)).find(|p| p.is_file()) {
            paths.push(found);
        }
    }

    let mut remaining = budget;
    let mut files = Vec::new();
    for path in paths {
        if remaining == 0 {
            break;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut content = trimmed.to_string();
        if content.len() > remaining {
            let mut cut = remaining;
            while !content.is_char_boundary(cut) {
                cut -= 1;
            }
            content.truncate(cut);
            content.push_str("\n... [truncated: instruction budget reached]");
        }
        remaining = remaining.saturating_sub(trimmed.len());
        files.push(InstructionFile { path, content });
    }
    files
}

/// Directories from the nearest ancestor containing `.git` down to `workspace`.
fn dirs_from_repo_root(workspace: &Path) -> Vec<PathBuf> {
    let start = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let ancestors: Vec<&Path> = start.ancestors().collect();
    let root_idx = ancestors
        .iter()
        .position(|d| d.join(".git").exists())
        .unwrap_or(0);
    ancestors[..=root_idx]
        .iter()
        .rev()
        .map(|d| d.to_path_buf())
        .collect()
}

pub fn render_instructions(files: &[InstructionFile]) -> Option<String> {
    if files.is_empty() {
        return None;
    }
    let mut out = String::from(
        "Project instructions (AGENTS.md). Follow them; deeper files override earlier ones:\n",
    );
    for f in files {
        out.push_str(&format!("\n--- {} ---\n{}\n", f.path.display(), f.content));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn collects_global_root_and_nested_files_in_order_with_override() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        let nested = repo.join("crates/app");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        let global = tmp.path().join("global.md");
        std::fs::write(&global, "global rule").unwrap();
        std::fs::write(repo.join("AGENTS.md"), "root rule").unwrap();
        std::fs::write(nested.join("AGENTS.md"), "ignored").unwrap();
        std::fs::write(nested.join("AGENTS.override.md"), "nested override").unwrap();
        std::fs::write(tmp.path().join("AGENTS.md"), "outside repo").unwrap();

        let files = load_instructions_with(Some(&global), &nested, MAX_PROJECT_DOC_BYTES);
        let contents: Vec<&str> = files.iter().map(|f| f.content.as_str()).collect();
        assert_eq!(
            contents,
            vec!["global rule", "root rule", "nested override"]
        );
        assert!(render_instructions(&files)
            .unwrap()
            .contains("nested override"));
    }

    #[test]
    fn truncates_to_budget() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(".git")).unwrap();
        std::fs::write(tmp.path().join("AGENTS.md"), "x".repeat(100)).unwrap();
        let files = load_instructions_with(None, tmp.path(), 10);
        assert!(files[0].content.starts_with("xxxxxxxxxx\n"));
        assert!(files[0].content.contains("truncated"));
    }
}
