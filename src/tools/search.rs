use async_trait::async_trait;
use globset::Glob;
use regex::Regex;
use serde_json::{json, Value};
use walkdir::{DirEntry, WalkDir};

use crate::tools::{Tool, ToolContext, ToolError, ToolOutput, ToolRisk};

const MAX_RESULTS: usize = 100;
const MAX_FILE_BYTES: u64 = 512 * 1024;

fn is_ignored_dir(entry: &DirEntry) -> bool {
    entry.file_type().is_dir()
        && matches!(
            entry.file_name().to_str(),
            Some(".git" | "target" | "node_modules" | ".venv" | "dist" | "build")
        )
}

pub struct SearchFilesTool;

#[async_trait]
impl Tool for SearchFilesTool {
    fn name(&self) -> &str {
        "search_files"
    }

    fn description(&self) -> &str {
        "Find files in the workspace matching a glob pattern (e.g. '**/*.rs', 'src/*.toml')."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern to match relative paths" }
            },
            "required": ["pattern"]
        })
    }

    fn risk_level(&self) -> ToolRisk {
        ToolRisk::ReadOnly
    }

    async fn execute(&self, ctx: &ToolContext, args: &Value) -> Result<ToolOutput, ToolError> {
        let pattern = args
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("Missing required string 'pattern'".into()))?;

        let matcher = Glob::new(pattern)
            .map_err(|e| ToolError::InvalidArgs(format!("Invalid glob pattern: {e}")))?
            .compile_matcher();

        let root = ctx.workspace_root.clone();
        let matches = tokio::task::spawn_blocking(move || {
            let mut found = Vec::new();
            let walker = WalkDir::new(&root)
                .into_iter()
                .filter_entry(|e| !is_ignored_dir(e));

            for entry in walker.flatten() {
                if !entry.file_type().is_file() {
                    continue;
                }
                if let Ok(rel) = entry.path().strip_prefix(&root) {
                    let normalized = rel.to_string_lossy().replace('\\', "/");
                    if matcher.is_match(&normalized) {
                        found.push(normalized);
                        if found.len() >= MAX_RESULTS {
                            break;
                        }
                    }
                }
            }
            found.sort();
            found
        })
        .await
        .map_err(|e| ToolError::Execution(e.to_string()))?;

        if matches.is_empty() {
            Ok(ToolOutput::ok("No files matched."))
        } else {
            Ok(ToolOutput::ok(matches.join("\n")))
        }
    }
}

pub struct GrepContentTool;

#[async_trait]
impl Tool for GrepContentTool {
    fn name(&self) -> &str {
        "grep_content"
    }

    fn description(&self) -> &str {
        "Search text/regex across workspace files and return matching file:line:content lines."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Regular expression pattern to search for" },
                "glob": { "type": "string", "description": "Optional glob filter for relative file paths (e.g. '**/*.rs')" }
            },
            "required": ["pattern"]
        })
    }

    fn risk_level(&self) -> ToolRisk {
        ToolRisk::ReadOnly
    }

    async fn execute(&self, ctx: &ToolContext, args: &Value) -> Result<ToolOutput, ToolError> {
        let pattern = args
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidArgs("Missing required string 'pattern'".into()))?;
        let regex = Regex::new(pattern)
            .map_err(|e| ToolError::InvalidArgs(format!("Invalid regex: {e}")))?;

        let glob_matcher = match args.get("glob").and_then(Value::as_str) {
            Some(g) => Some(
                Glob::new(g)
                    .map_err(|e| ToolError::InvalidArgs(format!("Invalid glob: {e}")))?
                    .compile_matcher(),
            ),
            None => None,
        };

        let root = ctx.workspace_root.clone();
        let hits = tokio::task::spawn_blocking(move || {
            let mut results = Vec::new();
            let walker = WalkDir::new(&root)
                .into_iter()
                .filter_entry(|e| !is_ignored_dir(e));

            for entry in walker.flatten() {
                if !entry.file_type().is_file() {
                    continue;
                }
                if entry.metadata().map(|m| m.len()).unwrap_or(0) > MAX_FILE_BYTES {
                    continue;
                }
                let Ok(rel) = entry.path().strip_prefix(&root) else {
                    continue;
                };
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                if let Some(ref matcher) = glob_matcher {
                    if !matcher.is_match(&rel_str) {
                        continue;
                    }
                }
                let Ok(content) = std::fs::read_to_string(entry.path()) else {
                    continue;
                };
                for (line_idx, line) in content.lines().enumerate() {
                    if regex.is_match(line) {
                        results.push(format!("{rel_str}:{}: {}", line_idx + 1, line.trim()));
                        if results.len() >= MAX_RESULTS {
                            return results;
                        }
                    }
                }
            }
            results
        })
        .await
        .map_err(|e| ToolError::Execution(e.to_string()))?;

        if hits.is_empty() {
            Ok(ToolOutput::ok("No matches found."))
        } else {
            Ok(ToolOutput::ok(hits.join("\n")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn search_and_grep_find_workspace_matches() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(
            tmp.path().join("src/lib.rs"),
            "pub fn answer() -> u32 { 42 }\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join("README.md"), "# Demo\n").unwrap();

        let ctx = ToolContext::new(tmp.path().to_path_buf(), 10);

        let files = SearchFilesTool
            .execute(&ctx, &json!({ "pattern": "**/*.rs" }))
            .await
            .unwrap();
        assert_eq!(files.content.trim(), "src/lib.rs");

        let grep = GrepContentTool
            .execute(&ctx, &json!({ "pattern": "fn answer" }))
            .await
            .unwrap();
        assert!(grep.content.contains("src/lib.rs:1: pub fn answer()"));
    }
}
