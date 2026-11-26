//! Context manager: a small, budgeted project brief injected before each turn.
//! Every item carries its provenance so the model can verify it.

use std::path::Path;
use std::process::Command;

use super::{KResult, KnowledgeStore};

#[derive(Debug, Clone, Copy)]
pub struct BriefOptions {
    /// Approximate token budget for the whole brief.
    pub budget_tokens: usize,
    pub max_memories: usize,
    pub max_code_hits: usize,
}

impl BriefOptions {
    pub fn standard() -> Self {
        Self {
            budget_tokens: 1500,
            max_memories: 8,
            max_code_hits: 6,
        }
    }

    pub fn saver() -> Self {
        Self {
            budget_tokens: 600,
            max_memories: 4,
            max_code_hits: 3,
        }
    }
}

/// Branch and modified files, or `None` outside a git repository.
pub fn git_state(workspace: &Path, max_files: usize) -> Option<String> {
    let out = Command::new("git")
        .args(["status", "--porcelain=v1", "--branch", "--untracked-files=normal"])
        .current_dir(workspace)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let branch = lines
        .next()
        .and_then(|l| l.strip_prefix("## "))
        .map(|b| b.split("...").next().unwrap_or(b).to_string())
        .unwrap_or_else(|| "?".into());
    let changed: Vec<String> = lines.map(|l| l.trim().to_string()).collect();
    let mut s = format!("branch {branch}");
    if changed.is_empty() {
        s.push_str(", clean");
    } else {
        let shown: Vec<&str> = changed.iter().take(max_files).map(String::as_str).collect();
        s.push_str(&format!(", {} changed: {}", changed.len(), shown.join(", ")));
        if changed.len() > max_files {
            s.push_str(", …");
        }
    }
    Some(s)
}

struct Budget {
    left: usize,
    out: Vec<String>,
}

impl Budget {
    fn push(&mut self, line: String) -> bool {
        let cost = line.len() / 4 + 1;
        if cost > self.left {
            return false;
        }
        self.left -= cost;
        self.out.push(line);
        true
    }
}

/// Refreshes the index and builds the brief for `prompt`. Returns `None` when empty.
pub fn build_brief(
    project: &mut KnowledgeStore,
    user: Option<&KnowledgeStore>,
    prompt: &str,
    opts: BriefOptions,
) -> KResult<Option<String>> {
    project.refresh()?;
    let mut b = Budget {
        left: opts.budget_tokens,
        out: Vec::new(),
    };

    if let Some(task) = project.active_task()? {
        b.push(format!("Active task #{}: {}", task.id, task.goal));
        if !task.plan.is_empty() {
            b.push(format!("Plan: {}", task.render_plan()));
        }
    }
    if let Some(git) = git_state(project.workspace(), 8) {
        b.push(format!("Git: {git}"));
    }

    let mut memories = project.search_memories(prompt, opts.max_memories)?;
    if let Some(user) = user {
        memories.extend(user.search_memories(prompt, opts.max_memories / 2 + 1)?);
    }
    memories.sort_by_key(|m| m.stale);
    if !memories.is_empty() && b.push("Memories (verify before relying on stale ones):".into()) {
        for m in memories.iter().take(opts.max_memories) {
            if !b.push(format!("- {}", m.render())) {
                break;
            }
        }
    }

    let mut code: Vec<String> = Vec::new();
    for word in prompt
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| w.len() >= 4 && w.chars().any(|c| c.is_uppercase() || c == '_'))
        .take(4)
    {
        for s in project.find_symbols(word, 2)? {
            code.push(format!("- {}:{} {} {}", s.path, s.line, s.kind, s.name));
        }
    }
    for hit in project.search_text(prompt, opts.max_code_hits, 6)? {
        code.push(format!("- {}:{}-{}", hit.path, hit.start_line, hit.end_line));
    }
    code.dedup();
    if !code.is_empty() && b.push("Possibly relevant code (index hint; read before editing):".into()) {
        for line in code.into_iter().take(opts.max_code_hits) {
            if !b.push(line) {
                break;
            }
        }
    }

    let recent: Vec<_> = project
        .recent_tasks(3)?
        .into_iter()
        .filter(|t| !t.active)
        .take(2)
        .collect();
    if !recent.is_empty() {
        b.push("Recent tasks:".into());
        for t in recent {
            let files = if t.files.is_empty() {
                String::new()
            } else {
                format!(" (files: {})", t.files.iter().take(4).cloned().collect::<Vec<_>>().join(", "))
            };
            if !b.push(format!("- #{} {}: {}{files}", t.id, t.status, t.goal)) {
                break;
            }
        }
    }

    if b.out.is_empty() {
        return Ok(None);
    }
    Ok(Some(format!("<project_context>\n{}\n</project_context>", b.out.join("\n"))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn brief_respects_budget_and_includes_provenance() {
        let tmp = TempDir::new().unwrap();
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::write(ws.join("src/cache.rs"), "pub struct TokenCache;\nimpl TokenCache {\n    pub fn evict(&self) {}\n}\n").unwrap();
        let mut s = KnowledgeStore::open_at(&tmp.path().join("k.db"), &ws).unwrap();
        s.refresh().unwrap();
        s.add_memory("project", "decision", "TokenCache evicts oldest entries first", &["src/cache.rs".into()])
            .unwrap();
        s.start_task("speed up cache").unwrap();

        let brief = build_brief(&mut s, None, "why does TokenCache evict entries?", BriefOptions::standard())
            .unwrap()
            .unwrap();
        assert!(brief.contains("Active task #1: speed up cache"));
        assert!(brief.contains("(src: src/cache.rs)"));
        assert!(brief.contains("src/cache.rs:1 struct TokenCache"));

        let tiny = BriefOptions { budget_tokens: 12, ..BriefOptions::saver() };
        let small = build_brief(&mut s, None, "TokenCache", tiny).unwrap().unwrap();
        assert!(small.len() < brief.len());
        assert!(small.len() / 4 <= 12 + 10);
    }
}
