pub mod diff;
pub mod fs;
pub mod knowledge;
pub mod patch;
pub mod sandbox;
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
    pub sandbox: sandbox::SandboxPolicy,
    pub knowledge: KnowledgeSettings,
}

/// Knowledge-tool settings carried with each tool call.
#[derive(Debug, Clone, Default)]
pub struct KnowledgeSettings {
    pub saver: bool,
    pub embedding: Option<crate::knowledge::EmbeddingClient>,
}

impl ToolContext {
    pub fn new(workspace_root: PathBuf, shell_timeout_secs: u64) -> Self {
        let canonical = workspace_root
            .canonicalize()
            .unwrap_or_else(|_| workspace_root.clone());
        Self {
            workspace_root: canonical,
            shell_timeout_secs,
            sandbox: sandbox::SandboxPolicy::default(),
            knowledge: KnowledgeSettings::default(),
        }
    }

    pub fn with_knowledge(mut self, knowledge: KnowledgeSettings) -> Self {
        self.knowledge = knowledge;
        self
    }

    pub fn with_sandbox(mut self, sandbox: sandbox::SandboxPolicy) -> Self {
        self.sandbox = sandbox;
        self
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
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    fn risk_level(&self) -> ToolRisk;

    /// Workspace files this call may create, modify or delete; used for checkpoints.
    fn touched_paths(&self, _ctx: &ToolContext, _args: &Value) -> Vec<PathBuf> {
        Vec::new()
    }

    fn preview(&self, _ctx: &ToolContext, args: &Value) -> Result<ToolPreview, ToolError> {
        Ok(ToolPreview {
            summary: format!("{}({})", self.name(), compact_args(args)),
            diff: None,
        })
    }

    async fn execute(&self, ctx: &ToolContext, args: &Value) -> Result<ToolOutput, ToolError>;
}

/// Longest prefix of `s` that is at most `max_bytes` long and ends on a char boundary.
pub fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut cut = max_bytes;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    &s[..cut]
}

pub fn compact_args(args: &Value) -> String {
    let serialized = args.to_string();
    if serialized.len() > 100 {
        format!("{}...", truncate_str(&serialized, 97))
    } else {
        serialized
    }
}

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
    order: Vec<String>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
            order: Vec::new(),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        if !self.tools.contains_key(&name) {
            self.order.push(name.clone());
        }
        self.tools.insert(name, tool);
    }

    pub fn default_registry() -> Self {
        let mut reg = Self::new();
        reg.register(Arc::new(fs::ReadFileTool));
        reg.register(Arc::new(fs::WriteFileTool));
        reg.register(Arc::new(fs::EditFileTool));
        reg.register(Arc::new(patch::ApplyPatchTool));
        reg.register(Arc::new(fs::ListDirTool));
        reg.register(Arc::new(search::SearchFilesTool));
        reg.register(Arc::new(search::GrepContentTool));
        reg.register(Arc::new(shell::RunCommandTool));
        reg.register_knowledge_tools(true);
        reg
    }

    fn register_knowledge_tools(&mut self, writable: bool) {
        self.register(Arc::new(knowledge::CodeSearchTool));
        self.register(Arc::new(knowledge::FindSymbolTool));
        self.register(Arc::new(knowledge::MemorySearchTool));
        if writable {
            self.register(Arc::new(knowledge::MemorySaveTool));
            self.register(Arc::new(knowledge::UpdatePlanTool));
        }
    }

    pub fn read_only_registry() -> Self {
        let mut reg = Self::new();
        reg.register_knowledge_tools(false);
        reg.register(Arc::new(fs::ReadFileTool));
        reg.register(Arc::new(fs::ListDirTool));
        reg.register(Arc::new(search::SearchFilesTool));
        reg.register(Arc::new(search::GrepContentTool));
        reg
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// Default tools plus extra (e.g. MCP) tools.
    pub fn with_extra_tools(extra: &[Arc<dyn Tool>]) -> Self {
        let mut reg = Self::default_registry();
        for tool in extra {
            reg.register(Arc::clone(tool));
        }
        reg
    }

    pub fn names(&self) -> Vec<String> {
        self.order.clone()
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

    #[test]
    fn truncation_respects_utf8_boundaries() {
        let text = "añ".repeat(80);
        assert_eq!(truncate_str(&text, 3), "añ");
        assert_eq!(truncate_str(&text, 2), "a");
        let summary = compact_args(&serde_json::json!({ "path": "ñ".repeat(120) }));
        assert!(summary.ends_with("..."));
    }
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
