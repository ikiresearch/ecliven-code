//! Agent tools backed by the project knowledge engine.

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::knowledge::{embed, KnowledgeStore, PlanStep};
use crate::tools::{Tool, ToolContext, ToolError, ToolOutput, ToolRisk};

async fn with_store<T: Send + 'static>(
    ctx: &ToolContext,
    refresh: bool,
    f: impl FnOnce(&mut KnowledgeStore) -> Result<T, String> + Send + 'static,
) -> Result<T, ToolError> {
    let ws = ctx.workspace_root.clone();
    tokio::task::spawn_blocking(move || {
        let mut store = KnowledgeStore::open(&ws)?;
        if refresh {
            store.refresh()?;
        }
        f(&mut store)
    })
    .await
    .map_err(|e| ToolError::Execution(e.to_string()))?
    .map_err(ToolError::Execution)
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

fn limit_arg(args: &Value, ctx: &ToolContext, normal: usize) -> usize {
    let default = if ctx.knowledge.saver { normal / 2 } else { normal };
    args.get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(default)
        .clamp(1, 50)
}

pub struct CodeSearchTool;

#[async_trait]
impl Tool for CodeSearchTool {
    fn name(&self) -> &str {
        "code_search"
    }

    fn description(&self) -> &str {
        "Search the project index (full text, plus semantic when embeddings are configured). Returns path:line ranges and short snippets; much cheaper than reading files. The index refreshes changed files first, but read a file before editing it."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Words, identifiers or a natural-language question" },
                "limit": { "type": "integer", "description": "Maximum results (default 10)" },
                "mode": { "type": "string", "enum": ["auto", "text", "semantic"], "description": "auto = text, plus semantic if available" }
            },
            "required": ["query"]
        })
    }

    fn risk_level(&self) -> ToolRisk {
        ToolRisk::ReadOnly
    }

    async fn execute(&self, ctx: &ToolContext, args: &Value) -> Result<ToolOutput, ToolError> {
        let query = str_arg(args, "query")
            .ok_or_else(|| ToolError::InvalidArgs("Missing required string 'query'".into()))?
            .to_string();
        let limit = limit_arg(args, ctx, 10);
        let mode = str_arg(args, "mode").unwrap_or("auto").to_string();
        let snippet = if ctx.knowledge.saver { 8 } else { 16 };
        let q = query.clone();
        let mut hits = if mode == "semantic" {
            with_store(ctx, true, |_| Ok(Vec::new())).await?
        } else {
            with_store(ctx, true, move |s| s.search_text(&q, limit, snippet)).await?
        };

        let mut note = String::new();
        let semantic_allowed = mode == "semantic" || (mode == "auto" && !ctx.knowledge.saver);
        match (&ctx.knowledge.embedding, semantic_allowed) {
            (Some(client), true) => match embed::semantic_search(client, &ctx.workspace_root, &query, limit).await {
                Ok(sem) => {
                    for h in sem {
                        if !hits.iter().any(|x| x.path == h.path && x.start_line == h.start_line) {
                            hits.push(h);
                        }
                    }
                }
                Err(e) => note = format!("\n(semantic search unavailable: {e})"),
            },
            (None, true) if mode == "semantic" => {
                note = "\n(semantic search is off: set [knowledge] embeddings = \"provider/model\")".into()
            }
            _ => {}
        }
        hits.truncate(limit);
        if hits.is_empty() {
            return Ok(ToolOutput::ok(format!("No indexed matches for '{query}'.{note}")));
        }
        let lines: Vec<String> = hits
            .iter()
            .map(|h| {
                if h.snippet.is_empty() {
                    format!("{}:{}-{} (semantic)", h.path, h.start_line, h.end_line)
                } else {
                    format!("{}:{}-{}  {}", h.path, h.start_line, h.end_line, h.snippet)
                }
            })
            .collect();
        Ok(ToolOutput::ok(format!("{}{note}", lines.join("\n"))))
    }
}

pub struct FindSymbolTool;

#[async_trait]
impl Tool for FindSymbolTool {
    fn name(&self) -> &str {
        "find_symbol"
    }

    fn description(&self) -> &str {
        "Find where a function, type, class or module is defined (exact or prefix match), plus files that import it and places that mention it. Pass 'path' instead to outline the symbols of one file."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Symbol name or prefix" },
                "path": { "type": "string", "description": "Workspace-relative file to outline" },
                "references": { "type": "boolean", "description": "Also list importers and mentions (default true)" }
            }
        })
    }

    fn risk_level(&self) -> ToolRisk {
        ToolRisk::ReadOnly
    }

    async fn execute(&self, ctx: &ToolContext, args: &Value) -> Result<ToolOutput, ToolError> {
        let limit = limit_arg(args, ctx, 20);
        if let Some(path) = str_arg(args, "path") {
            let rel = path.trim_start_matches("./").replace('\\', "/");
            let syms = with_store(ctx, true, move |s| s.symbols_in(&rel, limit * 5)).await?;
            if syms.is_empty() {
                return Ok(ToolOutput::ok(format!("No indexed symbols in {path}.")));
            }
            let out: Vec<String> = syms.iter().map(|s| format!("{}: {} {}", s.line, s.kind, s.signature)).collect();
            return Ok(ToolOutput::ok(out.join("\n")));
        }
        let name = str_arg(args, "name")
            .ok_or_else(|| ToolError::InvalidArgs("Pass 'name' or 'path'".into()))?
            .to_string();
        let refs = args.get("references").and_then(Value::as_bool).unwrap_or(true);
        let saver = ctx.knowledge.saver;
        let report = with_store(ctx, true, move |s| {
            let mut out = Vec::new();
            let defs = s.find_symbols(&name, limit)?;
            if defs.is_empty() {
                out.push(format!("No definition named '{name}' in the index."));
            }
            for d in &defs {
                out.push(format!("def {}:{} {} {}", d.path, d.line, d.kind, d.signature));
            }
            if refs {
                for (path, target) in s.importers(&name, limit)? {
                    out.push(format!("import {path} ({target})"));
                }
                for h in s.search_text(&name, limit, if saver { 6 } else { 10 })? {
                    if !defs.iter().any(|d| d.path == h.path && d.line >= h.start_line && d.line <= h.end_line) {
                        out.push(format!("mention {}:{}-{}  {}", h.path, h.start_line, h.end_line, h.snippet));
                    }
                }
            }
            Ok(out.join("\n"))
        })
        .await?;
        Ok(ToolOutput::ok(report))
    }
}

pub struct MemorySearchTool;

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search saved project and user memories (conventions, decisions, gotchas, preferences). Results show provenance; entries marked POSSIBLY STALE cite files that changed and must be re-verified."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Topic to recall; empty lists recent memories" },
                "limit": { "type": "integer" }
            }
        })
    }

    fn risk_level(&self) -> ToolRisk {
        ToolRisk::ReadOnly
    }

    async fn execute(&self, ctx: &ToolContext, args: &Value) -> Result<ToolOutput, ToolError> {
        let query = str_arg(args, "query").unwrap_or("").to_string();
        let limit = limit_arg(args, ctx, 10);
        let q = query.clone();
        let mut found = with_store(ctx, true, move |s| s.search_memories(&q, limit)).await?;
        let user = tokio::task::spawn_blocking(move || {
            KnowledgeStore::open_user().and_then(|u| u.search_memories(&query, limit))
        })
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
        found.extend(user);
        if found.is_empty() {
            return Ok(ToolOutput::ok("No matching memories."));
        }
        Ok(ToolOutput::ok(found.iter().map(|m| m.render()).collect::<Vec<_>>().join("\n")))
    }
}

pub struct MemorySaveTool;

#[async_trait]
impl Tool for MemorySaveTool {
    fn name(&self) -> &str {
        "memory_save"
    }

    fn description(&self) -> &str {
        "Save one durable, verified fact for future sessions (not task progress). Cite the files it depends on in 'sources' so it is flagged when they change. scope=user for personal preferences shared across projects."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string", "description": "One concise sentence" },
                "kind": { "type": "string", "enum": crate::knowledge::memory::MEMORY_KINDS },
                "scope": { "type": "string", "enum": ["project", "user"] },
                "sources": { "type": "array", "items": { "type": "string" }, "description": "Workspace-relative paths" }
            },
            "required": ["text"]
        })
    }

    fn risk_level(&self) -> ToolRisk {
        ToolRisk::ReadOnly
    }

    async fn execute(&self, ctx: &ToolContext, args: &Value) -> Result<ToolOutput, ToolError> {
        let text = str_arg(args, "text")
            .ok_or_else(|| ToolError::InvalidArgs("Missing required string 'text'".into()))?
            .to_string();
        let kind = str_arg(args, "kind").unwrap_or("fact").to_string();
        let scope = if str_arg(args, "scope") == Some("user") { "user" } else { "project" };
        let sources: Vec<String> = args
            .get("sources")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
            .unwrap_or_default();
        let id = if scope == "user" {
            tokio::task::spawn_blocking(move || KnowledgeStore::open_user()?.add_memory("user", &kind, &text, &[]))
                .await
                .map_err(|e| ToolError::Execution(e.to_string()))?
                .map_err(ToolError::Execution)?
        } else {
            with_store(ctx, true, move |s| s.add_memory("project", &kind, &text, &sources)).await?
        };
        let prefix = if scope == "user" { "u" } else { "m" };
        Ok(ToolOutput::ok(format!("Saved memory {prefix}{id}.")))
    }
}

pub struct UpdatePlanTool;

#[async_trait]
impl Tool for UpdatePlanTool {
    fn name(&self) -> &str {
        "update_plan"
    }

    fn description(&self) -> &str {
        "Record the goal and step plan of the current multi-step task (persisted across sessions). Send the full step list each time; set status=done when the task is finished."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "goal": { "type": "string", "description": "Task goal; starts a new task if it differs from the active one" },
                "steps": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "step": { "type": "string" },
                            "status": { "type": "string", "enum": ["pending", "in_progress", "done"] }
                        },
                        "required": ["step"]
                    }
                },
                "status": { "type": "string", "enum": ["active", "done", "abandoned"] }
            }
        })
    }

    fn risk_level(&self) -> ToolRisk {
        ToolRisk::ReadOnly
    }

    async fn execute(&self, ctx: &ToolContext, args: &Value) -> Result<ToolOutput, ToolError> {
        let goal = str_arg(args, "goal").map(String::from);
        let status = str_arg(args, "status").map(String::from);
        let steps: Vec<PlanStep> = match args.get("steps") {
            Some(v) => serde_json::from_value(v.clone())
                .map_err(|e| ToolError::InvalidArgs(format!("Invalid steps: {e}")))?,
            None => Vec::new(),
        };
        let task = with_store(ctx, false, move |s| s.update_plan(goal.as_deref(), &steps, status.as_deref())).await?;
        Ok(ToolOutput::ok(format!(
            "Task #{} ({}): {}\n{}",
            task.id,
            task.status,
            task.goal,
            task.render_plan()
        )))
    }
}
