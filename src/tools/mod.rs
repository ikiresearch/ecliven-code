pub mod diff;
pub mod fs;
pub mod search;
pub mod shell;

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use thiserror::Error;

use crate::ai::ToolSpec;
use crate::config::ApprovalMode;

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Invalid arguments: {0}")]
    InvalidArgs(String),

    #[error("Path '{0}' escapes the workspace root")]
    PathOutsideWorkspace(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Execution failed: {0}")]
    Execution(String),

    #[error("Tool '{0}' not found")]
    NotFound(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolRisk {
    ReadOnly,
    FileWrite,
    ShellExec,
}

impl ToolRisk {
    pub fn requires_approval(&self, mode: ApprovalMode) -> bool {
        match (self, mode) {
            (ToolRisk::ReadOnly, _) => false,
            (ToolRisk::FileWrite, ApprovalMode::Ask) => true,
            (ToolRisk::FileWrite, ApprovalMode::AutoEdit | ApprovalMode::AutoAll) => false,
            (ToolRisk::ShellExec, ApprovalMode::AutoAll) => false,
            (ToolRisk::ShellExec, ApprovalMode::Ask | ApprovalMode::AutoEdit) => true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub workspace_root: PathBuf,
    pub shell_timeout_secs: u64,
}

impl ToolContext {
    pub fn new(workspace_root: PathBuf, shell_timeout_secs: u64) -> Self {
        let canonical = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.clone());
        Self {
            workspace_root: canonical,
            shell_timeout_secs,
        }
    }

    pub fn resolve_path(&self, raw: &str) -> Result<PathBuf, ToolError> {
        let requested = Path::new(raw);
        let joined = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            self.workspace_root.join(requested)
        };

        let normalized = normalize_lexical(&joined);
        if !normalized.starts_with(&self.workspace_root) {
            return Err(ToolError::PathOutsideWorkspace(raw.to_string()));
        }

        if normalized.exists() {
            let canonical = normalized.canonicalize()?;
            if !canonical.starts_with(&self.workspace_root) {
                return Err(ToolError::PathOutsideWorkspace(raw.to_string()));
            }
            return Ok(canonical);
        }

        Ok(normalized)
    }
}

fn normalize_lexical(path: &Path) -> PathBuf {
    let mut stack: Vec<Component<'_>> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(stack.last(), Some(Component::Normal(_))) {
                    stack.pop();
                } else {
                    stack.push(comp);
                }
            }
            _ => stack.push(comp),
        }
    }
    stack.iter().collect()
}

#[derive(Debug, Clone)]
pub struct ToolPreview {
    pub summary: String,
    pub diff: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ToolOutput {
    pub content: String,
    pub diff: Option<String>,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            diff: None,
            is_error: false,
        }
    }

    pub fn with_diff(content: impl Into<String>, diff: String) -> Self {
        Self {
            content: content.into(),
            diff: Some(diff),
            is_error: false,
        }
    }

    pub fn err(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            diff: None,
            is_error: true,
        }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn input_schema(&self) -> Value;
    fn risk_level(&self) -> ToolRisk;

    fn preview(&self, _ctx: &ToolContext, args: &Value) -> Result<ToolPreview, ToolError> {
        Ok(ToolPreview {
            summary: format!("{}({})", self.name(), compact_args(args)),
            diff: None,
        })
    }

    async fn execute(&self, ctx: &ToolContext, args: &Value) -> Result<ToolOutput, ToolError>;
}

pub fn compact_args(args: &Value) -> String {
    let serialized = args.to_string();
    if serialized.len() > 100 {
        format!("{}...", &serialized[..97])
    } else {
        serialized
    }
}

pub struct ToolRegistry {
    tools: HashMap<&'static str, Arc<dyn Tool>>,
    order: Vec<&'static str>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            order: Vec::new(),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.name();
        if !self.tools.contains_key(name) {
            self.order.push(name);
        }
        self.tools.insert(name, tool);
    }

    pub fn default_registry() -> Self {
        let mut reg = Self::new();
        reg.register(Arc::new(fs::ReadFileTool));
        reg.register(Arc::new(fs::WriteFileTool));
        reg.register(Arc::new(fs::EditFileTool));
        reg.register(Arc::new(fs::ListDirTool));
        reg.register(Arc::new(search::SearchFilesTool));
        reg.register(Arc::new(search::GrepContentTool));
        reg.register(Arc::new(shell::RunCommandTool));
        reg
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    pub fn specs(&self) -> Vec<ToolSpec> {
        self.order
            .iter()
            .filter_map(|name| self.tools.get(name))
            .map(|t| ToolSpec {
                name: t.name().to_string(),
                description: t.description().to_string(),
                input_schema: t.input_schema(),
            })
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::default_registry()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn rejects_path_traversal_outside_workspace() {
        let tmp = TempDir::new().unwrap();
        let ctx = ToolContext::new(tmp.path().to_path_buf(), 10);

        let err = ctx.resolve_path("../../outside.txt").unwrap_err();
        assert!(matches!(err, ToolError::PathOutsideWorkspace(_)));
    }

    #[test]
    fn approval_matrix_matches_policy() {
        assert!(!ToolRisk::ReadOnly.requires_approval(ApprovalMode::Ask));
        assert!(ToolRisk::FileWrite.requires_approval(ApprovalMode::Ask));
        assert!(!ToolRisk::FileWrite.requires_approval(ApprovalMode::AutoEdit));
        assert!(ToolRisk::ShellExec.requires_approval(ApprovalMode::AutoEdit));
        assert!(!ToolRisk::ShellExec.requires_approval(ApprovalMode::AutoAll));
    }
}
