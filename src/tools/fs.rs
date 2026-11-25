use async_trait::async_trait;
use serde_json::{json, Value};

use crate::tools::diff::unified_diff;
use crate::tools::{Tool, ToolContext, ToolError, ToolOutput, ToolPreview, ToolRisk};

const MAX_READ_LINES: usize = 400;

pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "Read the contents of a file inside the workspace with line numbers. Supports optional 1-based offset and limit."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative path to the file inside the workspace" },
                "offset": { "type": "integer", "description": "1-based starting line number (default: 1)" },
                "limit": { "type": "integer", "description": "Maximum number of lines to read (default: 400)" }
            },
            "required": ["path"]
        })
    }

    fn risk_level(&self) -> ToolRisk {
        ToolRisk::ReadOnly
    }

    async fn execute(&self, ctx: &ToolContext, args: &Value) -> Result<ToolOutput, ToolError> {
        let raw_path = required_str(args, "path")?;
        let path = ctx.resolve_path(raw_path)?;

        let content = tokio::fs::read_to_string(&path).await?;
        let offset = args
            .get("offset")
            .and_then(Value::as_u64)
            .map(|n| n.saturating_sub(1) as usize)
            .unwrap_or(0);
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| (n as usize).min(MAX_READ_LINES))
            .unwrap_or(MAX_READ_LINES);

        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        let slice = lines.iter().skip(offset).take(limit);

        let mut out = String::new();
        for (i, line) in slice.enumerate() {
            out.push_str(&format!("{:>5} | {line}\n", offset + i + 1));
        }

        if offset + limit < total {
            out.push_str(&format!(
                "\n... ({} more lines; use offset/limit to read further)\n",
                total - (offset + limit)
            ));
        }

        Ok(ToolOutput::ok(out))
    }
}

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> &'static str {
        "Create a new file or overwrite an existing file inside the workspace."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative path to the file" },
                "content": { "type": "string", "description": "Full text content to write" }
            },
            "required": ["path", "content"]
        })
    }

    fn risk_level(&self) -> ToolRisk {
        ToolRisk::FileWrite
    }

    fn preview(&self, ctx: &ToolContext, args: &Value) -> Result<ToolPreview, ToolError> {
        let raw_path = required_str(args, "path")?;
        let new_content = required_str(args, "content")?;
        let path = ctx.resolve_path(raw_path)?;

        let old_content = if path.exists() {
            std::fs::read_to_string(&path).unwrap_or_default()
        } else {
            String::new()
        };

        let diff = unified_diff(
            &format!("a/{raw_path}"),
            &format!("b/{raw_path}"),
            &old_content,
            new_content,
        );

        Ok(ToolPreview {
            summary: format!("write_file: {raw_path}"),
            diff: Some(diff),
        })
    }

    async fn execute(&self, ctx: &ToolContext, args: &Value) -> Result<ToolOutput, ToolError> {
        let raw_path = required_str(args, "path")?;
        let new_content = required_str(args, "content")?;
        let path = ctx.resolve_path(raw_path)?;

        let old_content = if path.exists() {
            tokio::fs::read_to_string(&path).await.unwrap_or_default()
        } else {
            String::new()
        };

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, new_content).await?;

        let diff = unified_diff(
            &format!("a/{raw_path}"),
            &format!("b/{raw_path}"),
            &old_content,
            new_content,
        );

        Ok(ToolOutput::with_diff(
            format!("Wrote {} bytes to {raw_path}", new_content.len()),
            diff,
        ))
    }
}

pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn description(&self) -> &'static str {
        "Replace an exact unique string in an existing file. Fails if old_string is not found or matches multiple times unless replace_all is true."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative path to the file to edit" },
                "old_string": { "type": "string", "description": "Exact existing text to replace" },
                "new_string": { "type": "string", "description": "Replacement text" },
                "replace_all": { "type": "boolean", "description": "Replace all occurrences instead of requiring a unique match" }
            },
            "required": ["path", "old_string", "new_string"]
        })
    }

    fn risk_level(&self) -> ToolRisk {
        ToolRisk::FileWrite
    }

    fn preview(&self, ctx: &ToolContext, args: &Value) -> Result<ToolPreview, ToolError> {
        let raw_path = required_str(args, "path")?;
        let old_str = required_str(args, "old_string")?;
        let new_str = required_str(args, "new_string")?;
        let replace_all = args
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let path = ctx.resolve_path(raw_path)?;
        let current = std::fs::read_to_string(&path)?;
        let updated = compute_edit(&current, old_str, new_str, replace_all)?;

        let diff = unified_diff(
            &format!("a/{raw_path}"),
            &format!("b/{raw_path}"),
            &current,
            &updated,
        );

        Ok(ToolPreview {
            summary: format!("edit_file: {raw_path}"),
            diff: Some(diff),
        })
    }

    async fn execute(&self, ctx: &ToolContext, args: &Value) -> Result<ToolOutput, ToolError> {
        let raw_path = required_str(args, "path")?;
        let old_str = required_str(args, "old_string")?;
        let new_str = required_str(args, "new_string")?;
        let replace_all = args
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let path = ctx.resolve_path(raw_path)?;
        let current = tokio::fs::read_to_string(&path).await?;
        let updated = compute_edit(&current, old_str, new_str, replace_all)?;

        tokio::fs::write(&path, &updated).await?;

        let diff = unified_diff(
            &format!("a/{raw_path}"),
            &format!("b/{raw_path}"),
            &current,
            &updated,
        );

        Ok(ToolOutput::with_diff(
            format!("Edited {raw_path} successfully."),
            diff,
        ))
    }
}

fn compute_edit(
    current: &str,
    old_str: &str,
    new_str: &str,
    replace_all: bool,
) -> Result<String, ToolError> {
    if old_str.is_empty() {
        return Err(ToolError::InvalidArgs("old_string must not be empty".into()));
    }
    let count = current.matches(old_str).count();
    if count == 0 {
        return Err(ToolError::Execution(
            "old_string was not found in the file".into(),
        ));
    }
    if count > 1 && !replace_all {
        return Err(ToolError::Execution(format!(
            "old_string matched {count} times; provide more surrounding context or set replace_all=true"
        )));
    }

    if replace_all {
        Ok(current.replace(old_str, new_str))
    } else {
        Ok(current.replacen(old_str, new_str, 1))
    }
}

pub struct ListDirTool;

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &'static str {
        "list_dir"
    }

    fn description(&self) -> &'static str {
        "List files and subdirectories in a workspace directory."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Relative directory path (default: '.')" }
            }
        })
    }

    fn risk_level(&self) -> ToolRisk {
        ToolRisk::ReadOnly
    }

    async fn execute(&self, ctx: &ToolContext, args: &Value) -> Result<ToolOutput, ToolError> {
        let raw_path = args.get("path").and_then(Value::as_str).unwrap_or(".");
        let path = ctx.resolve_path(raw_path)?;

        let mut entries = Vec::new();
        let mut dir = tokio::fs::read_dir(&path).await?;
        while let Some(entry) = dir.next_entry().await? {
            let name = entry.file_name().to_string_lossy().to_string();
            if matches!(name.as_str(), ".git" | "target" | "node_modules") {
                continue;
            }
            let file_type = entry.file_type().await?;
            let suffix = if file_type.is_dir() { "/" } else { "" };
            entries.push(format!("{name}{suffix}"));
        }
        entries.sort();

        Ok(ToolOutput::ok(entries.join("\n")))
    }
}

fn required_str<'a>(args: &'a Value, field: &str) -> Result<&'a str, ToolError> {
    args.get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::InvalidArgs(format!("Missing required string field '{field}'")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn write_read_and_edit_file_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let ctx = ToolContext::new(tmp.path().to_path_buf(), 10);

        let write_out = WriteFileTool
            .execute(
                &ctx,
                &json!({ "path": "src/demo.rs", "content": "fn hello() {\n    println!(\"hi\");\n}\n" }),
            )
            .await
            .unwrap();
        assert!(write_out.diff.is_some());

        let read_out = ReadFileTool
            .execute(&ctx, &json!({ "path": "src/demo.rs" }))
            .await
            .unwrap();
        assert!(read_out.content.contains("1 | fn hello()"));

        let edit_out = EditFileTool
            .execute(
                &ctx,
                &json!({
                    "path": "src/demo.rs",
                    "old_string": "println!(\"hi\");",
                    "new_string": "println!(\"hello ecliven\");"
                }),
            )
            .await
            .unwrap();
        assert!(edit_out.diff.unwrap().contains("+    println!(\"hello ecliven\");"));

        let updated = std::fs::read_to_string(tmp.path().join("src/demo.rs")).unwrap();
        assert!(updated.contains("hello ecliven"));
    }

    #[tokio::test]
    async fn edit_file_rejects_ambiguous_match() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("dup.txt"), "foo\nbar\nfoo\n").unwrap();
        let ctx = ToolContext::new(tmp.path().to_path_buf(), 10);

        let err = EditFileTool
            .execute(
                &ctx,
                &json!({ "path": "dup.txt", "old_string": "foo", "new_string": "baz" }),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("matched 2 times"));
    }
}
