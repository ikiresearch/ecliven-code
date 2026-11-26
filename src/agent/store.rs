//! Persistent chat sessions as JSONL files (one record per line):
//! `<data dir>/ecliven-code/sessions/<id>.jsonl`.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::ai::Message;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub created_at: u64,
    pub workspace: PathBuf,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Record {
    Meta(SessionMeta),
    Message { message: Message },
    Replace { messages: Vec<Message> },
}

#[derive(Debug, Clone)]
pub struct SessionRecorder {
    path: PathBuf,
    meta: SessionMeta,
    started: bool,
}

impl SessionRecorder {
    pub fn id(&self) -> &str {
        &self.meta.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn append(&mut self, record: &Record) -> std::io::Result<()> {
        if !self.started {
            if let Record::Message { message } = record {
                if self.meta.title.is_empty() {
                    self.meta.title = message
                        .content
                        .lines()
                        .next()
                        .unwrap_or("")
                        .chars()
                        .take(80)
                        .collect();
                }
            }
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        if !self.started {
            writeln!(
                file,
                "{}",
                serde_json::to_string(&Record::Meta(self.meta.clone()))?
            )?;
            self.started = true;
        }
        writeln!(file, "{}", serde_json::to_string(record)?)
    }

    pub fn record_message(&mut self, message: &Message) {
        let _ = self.append(&Record::Message {
            message: message.clone(),
        });
    }

    pub fn record_replace(&mut self, messages: &[Message]) {
        let _ = self.append(&Record::Replace {
            messages: messages.to_vec(),
        });
    }
}

#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub meta: SessionMeta,
    pub path: PathBuf,
    pub updated_at: u64,
    pub message_count: usize,
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    dir: PathBuf,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn same_workspace(a: &Path, b: &Path) -> bool {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    canon(a) == canon(b)
}

impl SessionStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// `$ECLIVEN_SESSIONS_DIR`, else `<data dir>/ecliven-code/sessions`.
    pub fn default_store() -> Option<Self> {
        if let Some(dir) = std::env::var_os("ECLIVEN_SESSIONS_DIR").filter(|v| !v.is_empty()) {
            return Some(Self::new(PathBuf::from(dir)));
        }
        dirs::data_dir().map(|d| Self::new(d.join("ecliven-code").join("sessions")))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn create(
        &self,
        workspace: &Path,
        provider: &str,
        model: &str,
    ) -> std::io::Result<SessionRecorder> {
        std::fs::create_dir_all(&self.dir)?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let id = format!(
            "{:x}{:05x}",
            now_secs(),
            (nanos ^ std::process::id()) & 0xfffff
        );
        Ok(SessionRecorder {
            path: self.dir.join(format!("{id}.jsonl")),
            meta: SessionMeta {
                id,
                created_at: now_secs(),
                workspace: workspace
                    .canonicalize()
                    .unwrap_or_else(|_| workspace.to_path_buf()),
                provider: provider.to_string(),
                model: model.to_string(),
                title: String::new(),
            },
            started: false,
        })
    }

    /// Sessions newest first, optionally only those for `workspace`.
    pub fn list(&self, workspace: Option<&Path>) -> Vec<SessionSummary> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut out: Vec<SessionSummary> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
            .filter_map(|path| {
                let (meta, messages) = load_session(&path).ok()?;
                if workspace.is_some_and(|w| !same_workspace(w, &meta.workspace)) {
                    return None;
                }
                let updated_at = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(meta.created_at);
                Some(SessionSummary {
                    meta,
                    path,
                    updated_at,
                    message_count: messages.len(),
                })
            })
            .collect();
        out.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then(b.meta.id.cmp(&a.meta.id))
        });
        out
    }

    /// Finds a session by ID, unique ID prefix, or `last` (newest for the workspace).
    pub fn find(&self, query: &str, workspace: &Path) -> Result<SessionSummary, String> {
        if query.is_empty() || query == "last" || query == "latest" {
            return self
                .list(Some(workspace))
                .into_iter()
                .next()
                .ok_or_else(|| "No saved sessions for this workspace.".to_string());
        }
        let matches: Vec<SessionSummary> = self
            .list(None)
            .into_iter()
            .filter(|s| s.meta.id.starts_with(query))
            .collect();
        match matches.len() {
            0 => Err(format!("No session matches '{query}'.")),
            1 => Ok(matches.into_iter().next().unwrap()),
            n => Err(format!(
                "'{query}' matches {n} sessions; use a longer prefix."
            )),
        }
    }

    /// Reopens a saved session for appending.
    pub fn reopen(&self, summary: &SessionSummary) -> SessionRecorder {
        SessionRecorder {
            path: summary.path.clone(),
            meta: summary.meta.clone(),
            started: true,
        }
    }
}

pub fn load_session(path: &Path) -> Result<(SessionMeta, Vec<Message>), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut meta = None;
    let mut messages = Vec::new();
    for line in std::io::BufReader::new(file).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Record>(&line) {
            Ok(Record::Meta(m)) => meta = Some(m),
            Ok(Record::Message { message }) => messages.push(message),
            Ok(Record::Replace { messages: replaced }) => messages = replaced,
            Err(_) => continue,
        }
    }
    let meta = meta.ok_or_else(|| format!("{} has no session metadata", path.display()))?;
    Ok((meta, messages))
}

pub fn format_age(updated_at: u64) -> String {
    let secs = now_secs().saturating_sub(updated_at);
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", secs / 60),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::session::Session;
    use tempfile::TempDir;

    #[test]
    fn records_lists_finds_and_reloads_with_compaction() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::new(tmp.path().join("sessions"));
        let ws = tmp.path();

        let mut session = Session::new();
        session.attach_recorder(store.create(ws, "openai", "test/model").unwrap());
        session.push_user("fix the build\nplease");
        session.push_assistant("done", vec![]);
        session.replace_messages(vec![Message::user("summary")]);
        session.push_assistant("continuing", vec![]);

        let listed = store.list(Some(ws));
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].meta.title, "fix the build");
        assert_eq!(listed[0].message_count, 2);
        assert!(store.list(Some(&tmp.path().join("elsewhere"))).is_empty());

        let found = store.find("last", ws).unwrap();
        let prefix = &found.meta.id[..6];
        assert_eq!(store.find(prefix, ws).unwrap().meta.id, found.meta.id);

        let mut resumed = Session::from_messages(load_session(&found.path).unwrap().1);
        resumed.attach_recorder(store.reopen(&found));
        resumed.push_user("next");
        let (_, messages) = load_session(&found.path).unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].content, "summary");
    }
}
