//! Incremental code index: per-file hash, language and parser version, regex
//! symbol/relation extraction and full-text chunks. Only changed files are
//! re-parsed; memories that cite changed or deleted files are marked stale.

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

use regex::Regex;
use rusqlite::{params, OptionalExtension};
use walkdir::{DirEntry, WalkDir};

use super::{err, fts_query, hash_bytes, now, KResult, KnowledgeStore};

/// Bump when extraction rules change so every file is re-parsed.
pub const PARSER_VERSION: i64 = 1;
const MAX_FILE_BYTES: u64 = 512 * 1024;
const MAX_CHUNK_LINES: usize = 60;
const MIN_CHUNK_LINES: usize = 8;

const IGNORED_DIRS: &[&str] = &[
    ".git", "target", "node_modules", ".venv", "venv", "dist", "build", ".next", ".ecliven",
    "__pycache__", ".idea", ".vscode", "vendor", "coverage", ".cache",
];

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RefreshStats {
    pub scanned: usize,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub renamed: usize,
    pub stale_memories: usize,
}

impl RefreshStats {
    pub fn changed(&self) -> bool {
        self.added + self.updated + self.removed + self.renamed > 0
    }

    pub fn describe(&self) -> String {
        format!(
            "{} files scanned: {} added, {} updated, {} removed, {} renamed; {} memories marked stale",
            self.scanned, self.added, self.updated, self.removed, self.renamed, self.stale_memories
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodeHit {
    pub path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub snippet: String,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolHit {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub line: i64,
    pub signature: String,
}

pub fn language_for(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_str()?;
    if matches!(name, "Dockerfile" | "Makefile" | "Justfile") {
        return Some("build");
    }
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" | "mts" | "cts" => "typescript",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "cs" => "csharp",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "cpp",
        "rb" => "ruby",
        "php" => "php",
        "swift" => "swift",
        "vue" | "svelte" => "component",
        "md" | "mdx" => "markdown",
        "toml" | "yaml" | "yml" | "json" => "config",
        "sh" | "bash" | "zsh" | "ps1" => "shell",
        "sql" => "sql",
        "html" | "css" | "scss" => "web",
        _ => return None,
    })
}

struct Rules {
    symbols: Vec<(&'static str, Regex)>,
    imports: Vec<Regex>,
}

fn rules(language: &str) -> Option<&'static Rules> {
    static CACHE: OnceLock<HashMap<&'static str, Rules>> = OnceLock::new();
    let map = CACHE.get_or_init(|| {
        let r = |s: &str| Regex::new(s).expect("valid symbol regex");
        let mut m = HashMap::new();
        m.insert("rust", Rules {
            symbols: vec![
                ("fn", r(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+\S+\s+)?fn\s+([A-Za-z_]\w*)")),
                ("struct", r(r"^\s*(?:pub(?:\([^)]*\))?\s+)?struct\s+([A-Za-z_]\w*)")),
                ("enum", r(r"^\s*(?:pub(?:\([^)]*\))?\s+)?enum\s+([A-Za-z_]\w*)")),
                ("trait", r(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?trait\s+([A-Za-z_]\w*)")),
                ("type", r(r"^\s*(?:pub(?:\([^)]*\))?\s+)?type\s+([A-Za-z_]\w*)")),
                ("const", r(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const|static)\s+([A-Z_][A-Z0-9_]*)\s*:")),
                ("mod", r(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_]\w*)")),
                ("impl", r(r"^\s*impl(?:<[^>]*>)?\s+(?:[\w:<>, ]+\s+for\s+)?([A-Za-z_]\w*)")),
                ("macro", r(r"^\s*macro_rules!\s+([A-Za-z_]\w*)")),
            ],
            imports: vec![r(r"^\s*(?:pub(?:\([^)]*\))?\s+)?use\s+([^;{]+)")],
        });
        m.insert("python", Rules {
            symbols: vec![
                ("fn", r(r"^\s*(?:async\s+)?def\s+([A-Za-z_]\w*)")),
                ("class", r(r"^\s*class\s+([A-Za-z_]\w*)")),
            ],
            imports: vec![r(r"^\s*from\s+([\w.]+)\s+import"), r(r"^\s*import\s+([\w.]+)")],
        });
        let js = || Rules {
            symbols: vec![
                ("fn", r(r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\*?\s+([A-Za-z_$][\w$]*)")),
                ("class", r(r"^\s*(?:export\s+)?(?:default\s+)?(?:abstract\s+)?class\s+([A-Za-z_$][\w$]*)")),
                ("fn", r(r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*(?::[^=]+)?=\s*(?:async\s*)?(?:\([^)]*\)|[A-Za-z_$][\w$]*)\s*(?::[^=]+)?=>")),
                ("interface", r(r"^\s*(?:export\s+)?interface\s+([A-Za-z_$][\w$]*)")),
                ("type", r(r"^\s*(?:export\s+)?type\s+([A-Za-z_$][\w$]*)\s*(?:<[^>]*>)?\s*=")),
                ("enum", r(r"^\s*(?:export\s+)?(?:const\s+)?enum\s+([A-Za-z_$][\w$]*)")),
            ],
            imports: vec![
                r(r#"^\s*import\s+(?:[^'"]*\s+from\s+)?['"]([^'"]+)['"]"#),
                r(r#"^\s*export\s+[^'"]*\s+from\s+['"]([^'"]+)['"]"#),
                r(r#"require\(\s*['"]([^'"]+)['"]\s*\)"#),
            ],
        };
        m.insert("javascript", js());
        m.insert("typescript", js());
        m.insert("component", js());
        m.insert("go", Rules {
            symbols: vec![
                ("fn", r(r"^func\s+(?:\([^)]*\)\s*)?([A-Za-z_]\w*)")),
                ("type", r(r"^type\s+([A-Za-z_]\w*)")),
            ],
            imports: vec![r(r#"^\s*(?:import\s+)?(?:\w+\s+)?"([\w./-]+)"\s*$"#)],
        });
        let jvm = |extra: Vec<(&'static str, Regex)>| {
            let mut symbols = vec![
                ("class", r(r"^\s*(?:(?:public|private|protected|internal|abstract|final|sealed|static|data|open|partial)\s+)*(?:class|interface|enum|record|object|struct)\s+([A-Za-z_]\w*)")),
                ("fn", r(r"^\s*(?:(?:public|private|protected|internal|static|final|abstract|override|async|virtual|synchronized)\s+)+[\w<>\[\],.? ]+\s+([A-Za-z_]\w*)\s*\(")),
            ];
            symbols.extend(extra);
            Rules {
                symbols,
                imports: vec![r(r"^\s*import\s+(?:static\s+)?([\w.]+)"), r(r"^\s*using\s+([\w.]+)\s*;")],
            }
        };
        m.insert("java", jvm(vec![]));
        m.insert("csharp", jvm(vec![]));
        m.insert("kotlin", jvm(vec![("fn", r(r"^\s*(?:(?:private|public|internal|override|suspend|inline)\s+)*fun\s+(?:<[^>]*>\s*)?(?:[\w.]+\.)?([A-Za-z_]\w*)"))]));
        let c = || Rules {
            symbols: vec![
                ("type", r(r"^\s*(?:typedef\s+)?(?:struct|class|enum|union)\s+([A-Za-z_]\w*)")),
                ("fn", r(r"^(?:[A-Za-z_][\w:<>\*&]*\s+)+\**([A-Za-z_]\w*)\s*\([^;]*$")),
            ],
            imports: vec![r(r#"^\s*#\s*include\s+[<"]([^>"]+)[>"]"#)],
        };
        m.insert("c", c());
        m.insert("cpp", c());
        m.insert("ruby", Rules {
            symbols: vec![
                ("fn", r(r"^\s*def\s+(?:self\.)?([A-Za-z_]\w*[?!]?)")),
                ("class", r(r"^\s*(?:class|module)\s+([A-Z][\w:]*)")),
            ],
            imports: vec![r(r#"^\s*require(?:_relative)?\s+['"]([^'"]+)['"]"#)],
        });
        m.insert("php", Rules {
            symbols: vec![
                ("fn", r(r"^\s*(?:(?:public|private|protected|static|abstract|final)\s+)*function\s+([A-Za-z_]\w*)")),
                ("class", r(r"^\s*(?:abstract\s+|final\s+)?(?:class|interface|trait|enum)\s+([A-Za-z_]\w*)")),
            ],
            imports: vec![r(r"^\s*use\s+([\w\\]+)")],
        });
        m.insert("swift", Rules {
            symbols: vec![
                ("fn", r(r"^\s*(?:(?:public|private|internal|fileprivate|open|static|override|mutating)\s+)*func\s+([A-Za-z_]\w*)")),
                ("type", r(r"^\s*(?:(?:public|private|internal|open|final)\s+)*(?:class|struct|enum|protocol|extension|actor)\s+([A-Za-z_]\w*)")),
            ],
            imports: vec![r(r"^\s*import\s+(\w+)")],
        });
        m.insert("markdown", Rules {
            symbols: vec![("heading", r(r"^#{1,3}\s+(.+?)\s*#*\s*$"))],
            imports: vec![],
        });
        m
    });
    map.get(language)
}

const CONTROL_WORDS: &[&str] = &["if", "for", "while", "switch", "return", "else", "catch", "sizeof", "new", "delete"];

pub(crate) struct Parsed {
    pub symbols: Vec<(String, &'static str, i64, String)>,
    pub imports: Vec<String>,
    pub chunks: Vec<(i64, i64, String)>,
}

pub(crate) fn parse_file(language: &str, text: &str) -> Parsed {
    let lines: Vec<&str> = text.lines().collect();
    let mut symbols = Vec::new();
    let mut imports = Vec::new();
    if let Some(rules) = rules(language) {
        for (idx, line) in lines.iter().enumerate() {
            if line.len() > 400 {
                continue;
            }
            for (kind, re) in &rules.symbols {
                if let Some(name) = re.captures(line).and_then(|c| c.get(1)) {
                    let name = name.as_str().trim().to_string();
                    if CONTROL_WORDS.contains(&name.as_str()) {
                        continue;
                    }
                    let sig = crate::tools::truncate_str(line.trim(), 160).to_string();
                    symbols.push((name, *kind, idx as i64 + 1, sig));
                    break;
                }
            }
            for re in &rules.imports {
                if let Some(target) = re.captures(line).and_then(|c| c.get(1)) {
                    imports.push(target.as_str().trim().to_string());
                }
            }
        }
    }

    let starts: std::collections::HashSet<i64> = symbols.iter().map(|s| s.2).collect();
    let mut chunks = Vec::new();
    let mut start = 0usize;
    for idx in 0..=lines.len() {
        let len = idx - start;
        let at_symbol = starts.contains(&(idx as i64 + 1)) && len >= MIN_CHUNK_LINES;
        if idx == lines.len() || len >= MAX_CHUNK_LINES || at_symbol {
            if len > 0 {
                let body = lines[start..idx].join("\n");
                if !body.trim().is_empty() {
                    chunks.push((start as i64 + 1, idx as i64, body));
                }
            }
            start = idx;
        }
    }
    Parsed {
        symbols,
        imports,
        chunks,
    }
}

fn is_ignored(entry: &DirEntry) -> bool {
    entry.depth() > 0
        && entry.file_type().is_dir()
        && entry
            .file_name()
            .to_str()
            .map(|n| IGNORED_DIRS.contains(&n))
            .unwrap_or(false)
}

pub fn rel_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

struct Indexed {
    hash: String,
    size: i64,
    mtime: i64,
    parser_version: i64,
}

impl KnowledgeStore {
    /// Brings the index up to date with the workspace and returns what changed.
    pub fn refresh(&mut self) -> KResult<RefreshStats> {
        let root = self.workspace.clone();
        let mut stats = RefreshStats::default();
        let mut known: HashMap<String, Indexed> = HashMap::new();
        {
            let mut stmt = self
                .conn
                .prepare("SELECT path, hash, size, mtime, parser_version FROM files")
                .map_err(err)?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        Indexed {
                            hash: r.get(1)?,
                            size: r.get(2)?,
                            mtime: r.get(3)?,
                            parser_version: r.get(4)?,
                        },
                    ))
                })
                .map_err(err)?;
            for row in rows {
                let (p, i) = row.map_err(err)?;
                known.insert(p, i);
            }
        }

        let tx = self.conn.transaction().map_err(err)?;
        let mut added_hashes: HashMap<String, String> = HashMap::new();
        for entry in WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !is_ignored(e))
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            let Some(language) = language_for(path) else {
                continue;
            };
            let Ok(meta) = entry.metadata() else { continue };
            if meta.len() > MAX_FILE_BYTES {
                continue;
            }
            stats.scanned += 1;
            let rel = rel_path(&root, path);
            let size = meta.len() as i64;
            let mtime = meta
                .modified()
                .ok()
                .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let previous = known.remove(&rel);
            if let Some(prev) = &previous {
                if prev.size == size && prev.mtime == mtime && prev.parser_version == PARSER_VERSION {
                    continue;
                }
            }
            let Ok(bytes) = std::fs::read(path) else { continue };
            if bytes.iter().take(8192).any(|b| *b == 0) {
                continue;
            }
            let hash = hash_bytes(&bytes);
            if let Some(prev) = &previous {
                if prev.hash == hash && prev.parser_version == PARSER_VERSION {
                    tx.execute(
                        "UPDATE files SET size = ?2, mtime = ?3 WHERE path = ?1",
                        params![rel, size, mtime],
                    )
                    .map_err(err)?;
                    continue;
                }
            }
            let text = String::from_utf8_lossy(&bytes);
            let parsed = parse_file(language, &text);
            delete_file_rows(&tx, &rel)?;
            tx.execute(
                "INSERT OR REPLACE INTO files (path, hash, language, parser_version, size, mtime, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![rel, hash, language, PARSER_VERSION, size, mtime, now()],
            )
            .map_err(err)?;
            for (name, kind, line, sig) in &parsed.symbols {
                tx.execute(
                    "INSERT INTO symbols (path, name, kind, line, signature) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![rel, name, kind, line, sig],
                )
                .map_err(err)?;
            }
            for target in &parsed.imports {
                tx.execute(
                    "INSERT INTO relations (path, target, kind) VALUES (?1, ?2, 'import')",
                    params![rel, target],
                )
                .map_err(err)?;
            }
            for (start, end, body) in &parsed.chunks {
                tx.execute(
                    "INSERT INTO chunks (path, start_line, end_line, hash) VALUES (?1, ?2, ?3, ?4)",
                    params![rel, start, end, hash_bytes(body.as_bytes())],
                )
                .map_err(err)?;
                let id = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO chunks_fts (rowid, path, text) VALUES (?1, ?2, ?3)",
                    params![id, rel, body],
                )
                .map_err(err)?;
            }
            match previous {
                Some(prev) => {
                    stats.updated += 1;
                    stats.stale_memories += tx
                        .execute(
                            "UPDATE memories SET stale = 1, stale_reason = ?2
                             WHERE stale = 0 AND id IN (SELECT memory_id FROM memory_sources WHERE path = ?1 AND hash != ?3)",
                            params![rel, format!("{rel} changed since it was saved"), hash],
                        )
                        .map_err(err)?;
                    let _ = prev;
                }
                None => {
                    stats.added += 1;
                    added_hashes.insert(hash, rel);
                }
            }
        }

        for (rel, prev) in known {
            delete_file_rows(&tx, &rel)?;
            tx.execute("DELETE FROM files WHERE path = ?1", params![rel])
                .map_err(err)?;
            if let Some(new_path) = added_hashes.get(&prev.hash) {
                stats.renamed += 1;
                stats.added = stats.added.saturating_sub(1);
                tx.execute(
                    "UPDATE memory_sources SET path = ?2 WHERE path = ?1",
                    params![rel, new_path],
                )
                .map_err(err)?;
            } else {
                stats.removed += 1;
                stats.stale_memories += tx
                    .execute(
                        "UPDATE memories SET stale = 1, stale_reason = ?2
                         WHERE stale = 0 AND id IN (SELECT memory_id FROM memory_sources WHERE path = ?1)",
                        params![rel, format!("{rel} was deleted or excluded")],
                    )
                    .map_err(err)?;
            }
        }
        if stats.changed() {
            tx.execute(
                "DELETE FROM embeddings WHERE hash NOT IN (SELECT hash FROM chunks)",
                [],
            )
            .map_err(err)?;
        }
        tx.commit().map_err(err)?;
        Ok(stats)
    }

    pub fn file_hash(&self, rel: &str) -> KResult<Option<String>> {
        self.conn
            .query_row("SELECT hash FROM files WHERE path = ?1", params![rel], |r| r.get(0))
            .optional()
            .map_err(err)
    }

    /// Full-text search over indexed chunks, best first.
    pub fn search_text(&self, query: &str, limit: usize, snippet_tokens: usize) -> KResult<Vec<CodeHit>> {
        let Some(q) = fts_query(query, 12) else {
            return Ok(Vec::new());
        };
        let mut stmt = self
            .conn
            .prepare(
                "SELECT c.path, c.start_line, c.end_line,
                        snippet(chunks_fts, 1, '', '', '…', ?2), bm25(chunks_fts)
                 FROM chunks_fts JOIN chunks c ON c.id = chunks_fts.rowid
                 WHERE chunks_fts MATCH ?1 ORDER BY bm25(chunks_fts) LIMIT ?3",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map(params![q, snippet_tokens.clamp(4, 64) as i64, limit as i64], |r| {
                Ok(CodeHit {
                    path: r.get(0)?,
                    start_line: r.get(1)?,
                    end_line: r.get(2)?,
                    snippet: r.get::<_, String>(3)?.split_whitespace().collect::<Vec<_>>().join(" "),
                    score: -r.get::<_, f64>(4)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Definitions whose name matches exactly, then by prefix.
    pub fn find_symbols(&self, name: &str, limit: usize) -> KResult<Vec<SymbolHit>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT path, name, kind, line, signature FROM symbols
                 WHERE name = ?1 OR name LIKE ?2 ESCAPE '\\'
                 ORDER BY (name = ?1) DESC, length(name), path LIMIT ?3",
            )
            .map_err(err)?;
        let escaped = name.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        let rows = stmt
            .query_map(params![name, format!("{escaped}%"), limit as i64], |r| {
                Ok(SymbolHit {
                    path: r.get(0)?,
                    name: r.get(1)?,
                    kind: r.get(2)?,
                    line: r.get(3)?,
                    signature: r.get(4)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Files whose imports mention `needle` (module name or path fragment).
    pub fn importers(&self, needle: &str, limit: usize) -> KResult<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT path, target FROM relations WHERE target LIKE ?1 LIMIT ?2")
            .map_err(err)?;
        let rows = stmt
            .query_map(params![format!("%{needle}%"), limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    pub fn symbols_in(&self, rel: &str, limit: usize) -> KResult<Vec<SymbolHit>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path, name, kind, line, signature FROM symbols WHERE path = ?1 ORDER BY line LIMIT ?2")
            .map_err(err)?;
        let rows = stmt
            .query_map(params![rel, limit as i64], |r| {
                Ok(SymbolHit {
                    path: r.get(0)?,
                    name: r.get(1)?,
                    kind: r.get(2)?,
                    line: r.get(3)?,
                    signature: r.get(4)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }
}

fn delete_file_rows(tx: &rusqlite::Transaction<'_>, rel: &str) -> KResult<()> {
    tx.execute(
        "DELETE FROM chunks_fts WHERE rowid IN (SELECT id FROM chunks WHERE path = ?1)",
        params![rel],
    )
    .map_err(err)?;
    for table in ["chunks", "symbols", "relations"] {
        tx.execute(&format!("DELETE FROM {table} WHERE path = ?1"), params![rel])
            .map_err(err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_symbols_imports_and_chunks() {
        let src = "use crate::config::AppConfig;\n\npub struct Engine {\n    x: u32,\n}\n\nimpl Engine {\n    pub async fn run_turn(&self) {}\n}\n";
        let p = parse_file("rust", src);
        let names: Vec<_> = p.symbols.iter().map(|s| (s.0.as_str(), s.1)).collect();
        assert!(names.contains(&("Engine", "struct")));
        assert!(names.contains(&("Engine", "impl")));
        assert!(names.contains(&("run_turn", "fn")));
        assert_eq!(p.imports, vec!["crate::config::AppConfig"]);
        assert_eq!(p.chunks.len(), 1);

        let ts = "import { x } from './util';\nexport const handler = async (req) => {\n};\nexport interface Props {}\n";
        let p = parse_file("typescript", ts);
        assert_eq!(p.imports, vec!["./util"]);
        assert!(p.symbols.iter().any(|s| s.0 == "handler"));
        assert!(p.symbols.iter().any(|s| s.0 == "Props" && s.1 == "interface"));

        let py = "from app.models import User\nclass Service:\n    def run(self):\n        if x:\n            pass\n";
        let p = parse_file("python", py);
        assert_eq!(p.symbols.len(), 2);
        assert_eq!(p.imports, vec!["app.models"]);
    }
}
