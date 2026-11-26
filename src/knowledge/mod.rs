//! Project knowledge engine: a per-project SQLite store holding the code index
//! (files, symbols, relations, full-text chunks, optional embeddings), layered
//! memories with provenance, and task history. A separate user-level store keeps
//! knowledge shared across projects.

pub mod brief;
pub mod commands;
pub mod embed;
pub mod indexer;
pub mod memory;

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::Connection;
use sha2::{Digest, Sha256};

pub use brief::{build_brief, BriefOptions};
pub use embed::EmbeddingClient;
pub use indexer::{CodeHit, RefreshStats, SymbolHit, PARSER_VERSION};
pub use memory::{Memory, PlanStep, TaskRecord};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS files (
    path TEXT PRIMARY KEY,
    hash TEXT NOT NULL,
    language TEXT NOT NULL,
    parser_version INTEGER NOT NULL,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL,
    indexed_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS symbols (
    path TEXT NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL,
    line INTEGER NOT NULL,
    signature TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS symbols_name ON symbols(name);
CREATE INDEX IF NOT EXISTS symbols_path ON symbols(path);
CREATE TABLE IF NOT EXISTS relations (
    path TEXT NOT NULL,
    target TEXT NOT NULL,
    kind TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS relations_path ON relations(path);
CREATE INDEX IF NOT EXISTS relations_target ON relations(target);
CREATE TABLE IF NOT EXISTS chunks (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL,
    start_line INTEGER NOT NULL,
    end_line INTEGER NOT NULL,
    hash TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS chunks_path ON chunks(path);
CREATE INDEX IF NOT EXISTS chunks_hash ON chunks(hash);
CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(path, text);
CREATE TABLE IF NOT EXISTS embeddings (
    hash TEXT NOT NULL,
    model TEXT NOT NULL,
    vector BLOB NOT NULL,
    PRIMARY KEY (hash, model)
);
CREATE TABLE IF NOT EXISTS memories (
    id INTEGER PRIMARY KEY,
    scope TEXT NOT NULL,
    kind TEXT NOT NULL,
    text TEXT NOT NULL,
    stale INTEGER NOT NULL DEFAULT 0,
    stale_reason TEXT,
    created_at INTEGER NOT NULL,
    used_at INTEGER NOT NULL
);
CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(text);
CREATE TABLE IF NOT EXISTS memory_sources (
    memory_id INTEGER NOT NULL,
    path TEXT NOT NULL,
    hash TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS memory_sources_path ON memory_sources(path);
CREATE TABLE IF NOT EXISTS tasks (
    id INTEGER PRIMARY KEY,
    goal TEXT NOT NULL,
    status TEXT NOT NULL,
    plan TEXT NOT NULL DEFAULT '[]',
    files TEXT NOT NULL DEFAULT '[]',
    summary TEXT NOT NULL DEFAULT '',
    active INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
";

pub type KResult<T> = Result<T, String>;

pub(crate) fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub(crate) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().take(12).map(|b| format!("{b:02x}")).collect()
}

/// Directory holding knowledge databases (`ECLIVEN_KNOWLEDGE_DIR` overrides).
pub fn knowledge_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("ECLIVEN_KNOWLEDGE_DIR") {
        if !dir.trim().is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    dirs::data_dir().map(|d| d.join("ecliven-code").join("knowledge"))
}

pub fn project_db_path(workspace: &Path) -> Option<PathBuf> {
    let canonical = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let key = hash_bytes(canonical.to_string_lossy().as_bytes());
    let name = canonical
        .file_name()
        .map(|n| n.to_string_lossy().replace(|c: char| !c.is_ascii_alphanumeric(), "-"))
        .unwrap_or_else(|| "root".into());
    knowledge_dir().map(|d| d.join(format!("{name}-{key}.db")))
}

pub struct KnowledgeStore {
    pub(crate) conn: Connection,
    pub(crate) workspace: PathBuf,
}

impl KnowledgeStore {
    /// Opens (creating if needed) the store for `workspace`.
    pub fn open(workspace: &Path) -> KResult<Self> {
        let path = project_db_path(workspace).ok_or("no data directory available")?;
        Self::open_at(&path, workspace)
    }

    /// Opens the user-level store shared by every project.
    pub fn open_user() -> KResult<Self> {
        let dir = knowledge_dir().ok_or("no data directory available")?;
        Self::open_at(&dir.join("user.db"), Path::new(""))
    }

    pub fn open_at(db_path: &Path, workspace: &Path) -> KResult<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(err)?;
        }
        let conn = Connection::open(db_path).map_err(err)?;
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(err)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
            .map_err(err)?;
        conn.execute_batch(SCHEMA).map_err(err)?;
        Ok(Self {
            conn,
            workspace: workspace
                .canonicalize()
                .unwrap_or_else(|_| workspace.to_path_buf()),
        })
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    /// Removes every indexed file so the next refresh re-parses everything.
    pub fn clear_index(&self) -> KResult<()> {
        self.conn
            .execute_batch(
                "DELETE FROM files; DELETE FROM symbols; DELETE FROM relations;
                 DELETE FROM chunks; DELETE FROM chunks_fts;",
            )
            .map_err(err)
    }

    pub fn index_counts(&self) -> KResult<(i64, i64, i64)> {
        let count = |sql: &str| -> KResult<i64> {
            self.conn.query_row(sql, [], |r| r.get(0)).map_err(err)
        };
        Ok((
            count("SELECT COUNT(*) FROM files")?,
            count("SELECT COUNT(*) FROM symbols")?,
            count("SELECT COUNT(*) FROM chunks")?,
        ))
    }
}

/// Builds an FTS5 OR-query from free text (identifiers are split on punctuation).
pub(crate) fn fts_query(text: &str, max_terms: usize) -> Option<String> {
    let mut seen = Vec::new();
    for tok in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.chars().count() >= 2)
    {
        let tok = tok.to_lowercase();
        if STOPWORDS.contains(&tok.as_str()) || seen.contains(&tok) {
            continue;
        }
        seen.push(tok);
        if seen.len() >= max_terms {
            break;
        }
    }
    if seen.is_empty() {
        return None;
    }
    Some(
        seen.iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(" OR "),
    )
}

const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "this", "that", "from", "into", "are", "was", "how", "what",
    "can", "you", "our", "use", "add", "fix", "make", "please", "el", "la", "los", "las", "de",
    "del", "que", "en", "un", "una", "por", "para", "con", "es", "se", "lo", "al", "como", "y",
    "o", "mas", "más", "este", "esta", "hay", "su", "sus", "no", "si", "ya",
];
