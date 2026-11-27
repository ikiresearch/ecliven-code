//! Code graph over the index: imports resolved to workspace files, references
//! and call sites, impact analysis and a ranked repo map.

use std::collections::{HashMap, HashSet, VecDeque};

use rusqlite::params;

use super::indexer::SymbolHit;
use super::{err, KResult, KnowledgeStore};

const JS_EXTS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "vue", "svelte"];

fn parent_dir(p: &str) -> &str {
    p.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

fn join_norm(base: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

fn with_base(base: &str, p: &str) -> String {
    match (base.is_empty(), p.is_empty()) {
        (true, _) => p.to_string(),
        (_, true) => base.to_string(),
        _ => format!("{base}/{p}"),
    }
}

/// Workspace files an import of `target` in `from` refers to.
pub(crate) fn resolve(
    lang: &str,
    from: &str,
    target: &str,
    files: &HashSet<String>,
    go_dirs: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let exists = |c: &str| files.contains(c);
    match lang {
        "javascript" | "typescript" | "component" => {
            if !target.starts_with('.') {
                return Vec::new();
            }
            let Some(base) = join_norm(parent_dir(from), target) else {
                return Vec::new();
            };
            let mut candidates = vec![base.clone()];
            candidates.extend(JS_EXTS.iter().map(|e| format!("{base}.{e}")));
            candidates.extend(JS_EXTS.iter().map(|e| format!("{base}/index.{e}")));
            candidates.into_iter().find(|c| exists(c)).into_iter().collect()
        }
        "python" => {
            let dots = target.chars().take_while(|c| *c == '.').count();
            let segs: Vec<&str> = target[dots..].split('.').filter(|s| !s.is_empty()).collect();
            let bases: Vec<String> = if dots > 0 {
                let mut d = parent_dir(from).to_string();
                for _ in 1..dots {
                    d = parent_dir(&d).to_string();
                }
                vec![d]
            } else {
                vec![String::new(), "src".into(), parent_dir(from).to_string()]
            };
            for base in &bases {
                for k in (1..=segs.len()).rev() {
                    let p = with_base(base, &segs[..k].join("/"));
                    for c in [format!("{p}.py"), format!("{p}/__init__.py")] {
                        if exists(&c) {
                            return vec![c];
                        }
                    }
                }
            }
            Vec::new()
        }
        "rust" => {
            let t = target.replace(' ', "");
            let mut segs: Vec<&str> = t.split("::").filter(|s| !s.is_empty()).collect();
            let Some(i) = from.find("src/").or_else(|| from.starts_with("src/").then_some(0)) else {
                return Vec::new();
            };
            let src_root = from[..i + 3].to_string();
            let file = from.rsplit('/').next().unwrap_or("");
            let module_dir = if matches!(file, "mod.rs" | "lib.rs" | "main.rs") {
                parent_dir(from).to_string()
            } else {
                from.trim_end_matches(".rs").to_string()
            };
            let base = match segs.first().copied() {
                Some("crate") => {
                    segs.remove(0);
                    src_root
                }
                Some("self") => {
                    segs.remove(0);
                    module_dir
                }
                Some("super") => {
                    let mut d = module_dir;
                    while segs.first() == Some(&"super") {
                        segs.remove(0);
                        d = parent_dir(&d).to_string();
                    }
                    d
                }
                _ => return Vec::new(),
            };
            for k in (0..=segs.len()).rev() {
                let p = with_base(&base, &segs[..k].join("/"));
                let mut candidates = vec![format!("{p}.rs"), format!("{p}/mod.rs")];
                if k == 0 {
                    candidates.extend([format!("{p}/lib.rs"), format!("{p}/main.rs")]);
                }
                if let Some(c) = candidates.into_iter().find(|c| exists(c) && c != from) {
                    return vec![c];
                }
            }
            Vec::new()
        }
        "go" => go_dirs
            .iter()
            .filter(|(dir, _)| !dir.is_empty() && (target == dir.as_str() || target.ends_with(&format!("/{dir}"))))
            .max_by_key(|(dir, _)| dir.len())
            .map(|(_, fs)| fs.iter().take(20).cloned().collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Recomputes `file_deps` from the raw imports of every indexed file.
pub(crate) fn rebuild_deps(tx: &rusqlite::Transaction<'_>) -> KResult<()> {
    let files: Vec<(String, String)> = {
        let mut stmt = tx.prepare("SELECT path, language FROM files").map_err(err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)?
    };
    let lang: HashMap<&str, &str> = files.iter().map(|(p, l)| (p.as_str(), l.as_str())).collect();
    let set: HashSet<String> = files.iter().map(|(p, _)| p.clone()).collect();
    let mut go_dirs: HashMap<String, Vec<String>> = HashMap::new();
    for (p, l) in &files {
        if l == "go" && !p.ends_with("_test.go") {
            go_dirs.entry(parent_dir(p).to_string()).or_default().push(p.clone());
        }
    }
    let imports: Vec<(String, String)> = {
        let mut stmt = tx.prepare("SELECT path, target FROM relations").map_err(err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)?
    };
    tx.execute("DELETE FROM file_deps", []).map_err(err)?;
    let mut insert = tx
        .prepare("INSERT OR IGNORE INTO file_deps (path, dep) VALUES (?1, ?2)")
        .map_err(err)?;
    for (path, target) in &imports {
        let Some(l) = lang.get(path.as_str()) else { continue };
        for dep in resolve(l, path, target, &set, &go_dirs) {
            if &dep != path {
                insert.execute(params![path, dep]).map_err(err)?;
            }
        }
    }
    Ok(())
}

pub fn is_test_path(p: &str) -> bool {
    let lower = p.to_ascii_lowercase();
    lower.starts_with("tests/")
        || lower.contains("/tests/")
        || lower.contains("/test/")
        || lower.contains("__tests__")
        || lower.ends_with("_test.go")
        || lower.ends_with("_test.py")
        || lower.rsplit('/').next().is_some_and(|f| f.starts_with("test_") || f.contains(".test.") || f.contains(".spec."))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefHit {
    pub path: String,
    pub line: i64,
    pub caller: String,
    pub kind: String,
}

impl KnowledgeStore {
    pub(crate) fn symbol_rows(&self, sql_where: &str, args: &[&dyn rusqlite::ToSql]) -> KResult<Vec<SymbolHit>> {
        let sql = format!(
            "SELECT path, name, kind, line, signature, end_line, parent FROM symbols WHERE {sql_where}"
        );
        let mut stmt = self.conn.prepare(&sql).map_err(err)?;
        let rows = stmt
            .query_map(args, |r| {
                Ok(SymbolHit {
                    path: r.get(0)?,
                    name: r.get(1)?,
                    kind: r.get(2)?,
                    line: r.get(3)?,
                    signature: r.get(4)?,
                    end_line: r.get(5)?,
                    parent: r.get(6)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Exact definitions of `name` (`Type::method` or `Type.method` narrows by parent).
    pub fn definitions(&self, name: &str, path: Option<&str>, limit: usize) -> KResult<Vec<SymbolHit>> {
        let (parent, name) = match name.rsplit_once("::").or_else(|| name.rsplit_once('.')) {
            Some((p, n)) => (p.rsplit(['.', ':']).next().unwrap_or(p).to_string(), n.to_string()),
            None => (String::new(), name.to_string()),
        };
        self.symbol_rows(
            "name = ?1 AND (?2 = '' OR parent = ?2) AND (?3 = '' OR path = ?3)
             ORDER BY kind = 'impl', path, line LIMIT ?4",
            &[&name, &parent, &path.unwrap_or(""), &(limit as i64)],
        )
    }

    pub fn references(&self, name: &str, kind: Option<&str>, limit: usize) -> KResult<Vec<RefHit>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT path, line, caller, kind FROM refs WHERE name = ?1 AND (?2 = '' OR kind = ?2)
                 ORDER BY path, line LIMIT ?3",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map(params![name, kind.unwrap_or(""), limit as i64], |r| {
                Ok(RefHit { path: r.get(0)?, line: r.get(1)?, caller: r.get(2)?, kind: r.get(3)? })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Functions called from inside `caller`, as `(callee, path, line)`.
    pub fn callees(&self, caller: &str, limit: usize) -> KResult<Vec<(String, String, i64)>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT name, path, MIN(line) FROM refs WHERE caller = ?1 AND kind = 'call'
                 GROUP BY name, path ORDER BY path, MIN(line) LIMIT ?2",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map(params![caller, limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    pub fn dependencies(&self, path: &str) -> KResult<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT dep FROM file_deps WHERE path = ?1 ORDER BY dep")
            .map_err(err)?;
        let rows = stmt.query_map(params![path], |r| r.get(0)).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Files that import `paths` directly or transitively, with their distance.
    pub fn dependents(&self, paths: &[String], depth: usize, limit: usize) -> KResult<Vec<(String, usize)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM file_deps WHERE dep = ?1")
            .map_err(err)?;
        let mut seen: HashSet<String> = paths.iter().cloned().collect();
        let mut queue: VecDeque<(String, usize)> = paths.iter().map(|p| (p.clone(), 0)).collect();
        let mut out = Vec::new();
        while let Some((p, d)) = queue.pop_front() {
            if d >= depth || out.len() >= limit {
                continue;
            }
            let rows: Vec<String> = stmt
                .query_map(params![p], |r| r.get(0))
                .map_err(err)?
                .collect::<Result<_, _>>()
                .map_err(err)?;
            for next in rows {
                if seen.insert(next.clone()) {
                    out.push((next.clone(), d + 1));
                    queue.push_back((next, d + 1));
                }
            }
        }
        out.truncate(limit);
        Ok(out)
    }

    /// Most central files by PageRank over the dependency graph.
    pub fn repo_map(&self, limit: usize) -> KResult<Vec<(String, f64)>> {
        let edges: Vec<(String, String)> = {
            let mut stmt = self.conn.prepare("SELECT path, dep FROM file_deps").map_err(err)?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map_err(err)?;
            rows.collect::<Result<_, _>>().map_err(err)?
        };
        if edges.is_empty() {
            return Ok(Vec::new());
        }
        let mut ids: HashMap<&str, usize> = HashMap::new();
        for (a, b) in &edges {
            let n = ids.len();
            ids.entry(a).or_insert(n);
            let n = ids.len();
            ids.entry(b).or_insert(n);
        }
        let n = ids.len();
        let mut out_deg = vec![0usize; n];
        for (a, _) in &edges {
            out_deg[ids[a.as_str()]] += 1;
        }
        let mut rank = vec![1.0 / n as f64; n];
        for _ in 0..20 {
            let mut next = vec![0.15 / n as f64; n];
            for (a, b) in &edges {
                let ia = ids[a.as_str()];
                next[ids[b.as_str()]] += 0.85 * rank[ia] / out_deg[ia] as f64;
            }
            rank = next;
        }
        let mut scored: Vec<(String, f64)> = ids
            .iter()
            .filter(|(p, _)| !is_test_path(p))
            .map(|(p, i)| (p.to_string(), rank[*i]))
            .collect();
        scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        scored.truncate(limit);
        Ok(scored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn resolves_imports_per_language() {
        let none = HashMap::new();
        let files = set(&["src/ui/button.tsx", "src/ui/index.ts", "app/models/user.py", "src/config/mod.rs", "src/tools/fs.rs", "src/tools/mod.rs", "src/lib.rs"]);
        assert_eq!(resolve("typescript", "src/app.ts", "./ui/button", &files, &none), vec!["src/ui/button.tsx"]);
        assert_eq!(resolve("typescript", "src/app.ts", "./ui", &files, &none), vec!["src/ui/index.ts"]);
        assert!(resolve("typescript", "src/app.ts", "react", &files, &none).is_empty());
        assert_eq!(resolve("python", "app/main.py", "app.models.user", &files, &none), vec!["app/models/user.py"]);
        assert_eq!(resolve("python", "app/models/x.py", ".user", &files, &none), vec!["app/models/user.py"]);
        assert_eq!(resolve("rust", "src/agent/mod.rs", "crate::config::AppConfig", &files, &none), vec!["src/config/mod.rs"]);
        assert_eq!(resolve("rust", "src/tools/mod.rs", "self::fs", &files, &none), vec!["src/tools/fs.rs"]);
        assert_eq!(resolve("rust", "src/tools/fs.rs", "super::ToolError", &files, &none), vec!["src/tools/mod.rs"]);
        assert!(resolve("rust", "src/tools/fs.rs", "std::fs", &files, &none).is_empty());
        let mut go = HashMap::new();
        go.insert("internal/db".to_string(), vec!["internal/db/conn.go".to_string()]);
        assert_eq!(resolve("go", "main.go", "example.com/app/internal/db", &files, &go), vec!["internal/db/conn.go"]);
        assert!(is_test_path("tests/agent.rs") && is_test_path("src/a.test.ts") && !is_test_path("src/attest.rs"));
    }

    #[test]
    fn refresh_builds_references_call_graph_and_dependents() {
        let ws = tempfile::TempDir::new().unwrap();
        let root = ws.path();
        std::fs::create_dir_all(root.join("src/tools")).unwrap();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub mod tools;\npub mod engine;\n").unwrap();
        std::fs::write(root.join("src/tools/mod.rs"), "pub fn sanitize(s: &str) -> String { s.trim().into() }\n").unwrap();
        std::fs::write(
            root.join("src/engine.rs"),
            "use crate::tools::sanitize;\npub struct Engine;\nimpl Engine {\n    pub fn run(&self) -> String {\n        sanitize(\" x \")\n    }\n}\n",
        )
        .unwrap();
        std::fs::write(root.join("tests/engine_test.rs"), "use app::engine::Engine;\n#[test]\nfn t() { Engine.run(); }\n").unwrap();
        let db = tempfile::TempDir::new().unwrap();
        let mut store = KnowledgeStore::open_at(&db.path().join("k.db"), root).unwrap();
        store.refresh().unwrap();

        let run = store.definitions("Engine::run", None, 5).unwrap();
        assert_eq!((run[0].path.as_str(), run[0].line, run[0].end_line), ("src/engine.rs", 4, 6));
        let refs = store.references("sanitize", Some("call"), 10).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!((refs[0].path.as_str(), refs[0].caller.as_str()), ("src/engine.rs", "run"));
        assert!(store.callees("run", 10).unwrap().iter().any(|c| c.0 == "sanitize"));
        assert_eq!(store.dependencies("src/engine.rs").unwrap(), vec!["src/tools/mod.rs"]);
        let dependents = store.dependents(&["src/tools/mod.rs".to_string()], 2, 10).unwrap();
        assert!(dependents.iter().any(|(p, _)| p == "src/engine.rs"));
        assert!(store.repo_map(3).unwrap().iter().any(|(p, _)| p == "src/tools/mod.rs"));
    }
}
