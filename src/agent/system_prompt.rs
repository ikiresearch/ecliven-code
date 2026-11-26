use std::path::Path;

pub fn build_system_prompt(workspace_root: &Path) -> String {
    let os = std::env::consts::OS;
    let entries = collect_top_level_entries(workspace_root, 30);
    let listing = if entries.is_empty() {
        "(empty workspace)".to_string()
    } else {
        entries.join(", ")
    };

    let mut prompt = format!(
        "You are Ecliven Code, a fast, precise terminal AI coding agent built in Rust.\n\
         \n\
         Environment:\n\
         - Operating system: {os}\n\
         - Workspace root: {}\n\
         - Top-level entries: {listing}\n\
         \n\
         Guidelines:\n\
         1. Inspect files before editing them using read_file, list_dir, search_files, or grep_content.\n\
         2. Prefer minimal, targeted changes with edit_file (one change) or apply_patch (several hunks or files) rather than rewriting entire files.\n\
         3. Never fabricate file contents or command output; always verify with tools.\n\
         4. After modifying code, run relevant build/test commands with run_command when helpful.\n\
         5. Keep responses concise, accurate, and easy to scan in a terminal.\n\
         6. Subdirectories may contain their own AGENTS.md; read it before editing files there.",
        workspace_root.display()
    );
    let files = crate::agent::instructions::load_project_instructions(workspace_root);
    if let Some(rendered) = crate::agent::instructions::render_instructions(&files) {
        prompt.push_str("\n\n");
        prompt.push_str(&rendered);
    }
    prompt
}

fn collect_top_level_entries(workspace_root: &Path, limit: usize) -> Vec<String> {
    let Ok(read_dir) = std::fs::read_dir(workspace_root) else {
        return Vec::new();
    };

    let mut items: Vec<String> = read_dir
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            if matches!(name.as_str(), ".git" | "target" | "node_modules") {
                return None;
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Some(if is_dir { format!("{name}/") } else { name })
        })
        .collect();

    items.sort();
    items.truncate(limit);
    items
}
