//! Skills: reusable instructions in `SKILL.md` files. Only each skill's name and
//! description go into the system prompt; the body is loaded on demand with
//! the `load_skill` tool, so unused skills cost almost no tokens.

use std::path::{Path, PathBuf};

const MAX_SKILL_BYTES: u64 = 64 * 1024;
const MAX_LISTED: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

/// Project skills first (`.ecliven/skills`, `.agents/skills`), then user skills.
pub fn skill_dirs(workspace: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![
        workspace.join(".ecliven").join("skills"),
        workspace.join(".agents").join("skills"),
    ];
    if let Some(cfg) = dirs::config_dir() {
        dirs.push(cfg.join("ecliven-code").join("skills"));
    }
    dirs
}

fn parse_frontmatter(text: &str) -> (Option<String>, Option<String>, &str) {
    let Some(rest) = text.strip_prefix("---") else {
        return (None, None, text);
    };
    let Some(end) = rest.find("\n---") else {
        return (None, None, text);
    };
    let (mut name, mut description) = (None, None);
    for line in rest[..end].lines() {
        if let Some((k, v)) = line.split_once(':') {
            let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
            match k.trim() {
                "name" if !v.is_empty() => name = Some(v),
                "description" if !v.is_empty() => description = Some(v),
                _ => {}
            }
        }
    }
    let body = rest[end + 4..].trim_start_matches(['-', '\r', '\n']);
    (name, description, body)
}

fn read_skill(path: &Path) -> Option<(Skill, String)> {
    if std::fs::metadata(path).ok()?.len() > MAX_SKILL_BYTES {
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    let (name, description, body) = parse_frontmatter(&text);
    let fallback = path.parent()?.file_name()?.to_string_lossy().to_string();
    let name = name.unwrap_or(fallback);
    let description = description.unwrap_or_else(|| {
        body.lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .unwrap_or("")
            .chars()
            .take(160)
            .collect()
    });
    Some((Skill { name, description, path: path.to_path_buf() }, body.to_string()))
}

/// Skills found in `<dir>/<skill>/SKILL.md`; earlier dirs win on name clashes.
pub fn discover(workspace: &Path) -> Vec<Skill> {
    let mut out: Vec<Skill> = Vec::new();
    for dir in skill_dirs(workspace) {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path().join("SKILL.md"))
            .filter(|p| p.is_file())
            .collect();
        paths.sort();
        for p in paths {
            if let Some((skill, _)) = read_skill(&p) {
                if !out.iter().any(|s| s.name == skill.name) {
                    out.push(skill);
                }
            }
        }
    }
    out
}

pub fn load_body(workspace: &Path, name: &str) -> Option<(Skill, String)> {
    let skill = discover(workspace).into_iter().find(|s| s.name.eq_ignore_ascii_case(name))?;
    read_skill(&skill.path)
}

/// Section for the system prompt; `None` when there are no skills.
pub fn render_index(skills: &[Skill]) -> Option<String> {
    if skills.is_empty() {
        return None;
    }
    let mut out = String::from(
        "Skills available (call load_skill with the name before a task that matches its description; follow the loaded instructions):\n",
    );
    for s in skills.iter().take(MAX_LISTED) {
        out.push_str(&format!("- {}: {}\n", s.name, s.description));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_project_skills_with_frontmatter_and_fallbacks() {
        let ws = tempfile::TempDir::new().unwrap();
        let a = ws.path().join(".ecliven/skills/release");
        let b = ws.path().join(".agents/skills/db-migrations");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("SKILL.md"), "---\nname: release\ndescription: \"Cut a release\"\n---\n# Steps\n1. Bump version\n").unwrap();
        std::fs::write(b.join("SKILL.md"), "# Migrations\nAlways add a down migration.\n").unwrap();
        let skills = discover(ws.path());
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"release") && names.contains(&"db-migrations"));
        let mig = skills.iter().find(|s| s.name == "db-migrations").unwrap();
        assert_eq!(mig.description, "Always add a down migration.");
        let (_, body) = load_body(ws.path(), "RELEASE").unwrap();
        assert!(body.starts_with("# Steps"));
        let index = render_index(&skills).unwrap();
        assert!(index.contains("- release: Cut a release"));
        assert!(render_index(&[]).is_none());
    }
}
