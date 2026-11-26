//! Persistent chat sessions as JSONL files (one record per line):
//! `<data dir>/ecliven-code/sessions/<id>.jsonl`.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::agent::checkpoint::{Checkpoint, FileOriginal};
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
    Message {
        message: Message,
    },
    Replace {
        messages: Vec<Message>,
    },
    CheckpointFile {
        seq: u64,
        label: String,
        path: PathBuf,
        original: FileOriginal,
    },
    CheckpointPop {
        seq: u64,
    },
}

/// Everything restored from a session file.
#[derive(Debug, Clone)]
pub struct LoadedSession {
    pub meta: SessionMeta,
    pub messages: Vec<Message>,
    pub checkpoints: Vec<Checkpoint>,
}

#[derive(Debug, Clone)]
pub struct SessionRecorder {
    path: PathBuf,
    meta: SessionMeta,
    started: bool,
    last_error: Option<String>,
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

    fn append_or_remember(&mut self, record: &Record) {
        if let Err(err) = self.append(record) {
            self.last_error = Some(format!("{}: {err}", self.path.display()));
        }
    }

    /// The latest write failure, cleared once read.
    pub fn take_error(&mut self) -> Option<String> {
        self.last_error.take()
    }

    pub fn record_message(&mut self, message: &Message) {
        self.append_or_remember(&Record::Message {
            message: message.clone(),
        });
    }

    pub fn record_replace(&mut self, messages: &[Message]) {
        self.append_or_remember(&Record::Replace {
            messages: messages.to_vec(),
        });
    }

    pub fn record_checkpoint_file(
        &mut self,
        seq: u64,
        label: &str,
        path: &Path,
        original: &FileOriginal,
    ) {
        self.append_or_remember(&Record::CheckpointFile {
            seq,
            label: label.to_string(),
            path: path.to_path_buf(),
            original: original.clone(),
        });
    }

    pub fn record_checkpoint_pop(&mut self, seq: u64) {
        self.append_or_remember(&Record::CheckpointPop { seq });
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
            last_error: None,
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
                let LoadedSession { meta, messages, .. } = load_session(&path).ok()?;
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
            last_error: None,
        }
    }
}

pub fn load_session(path: &Path) -> Result<LoadedSession, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut meta = None;
    let mut messages = Vec::new();
    let mut checkpoints: Vec<Checkpoint> = Vec::new();
    for line in std::io::BufReader::new(file).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Record>(&line) {
            Ok(Record::Meta(m)) => meta = Some(m),
            Ok(Record::Message { message }) => messages.push(message),
            Ok(Record::Replace { messages: replaced }) => messages = replaced,
            Ok(Record::CheckpointFile {
                seq,
                label,
                path,
                original,
            }) => {
                if checkpoints.last().map(|c| c.seq) != Some(seq) {
                    checkpoints.push(Checkpoint::new(seq, label));
                }
                if let Some(cp) = checkpoints.last_mut() {
                    cp.insert(path, original);
                }
            }
            Ok(Record::CheckpointPop { seq }) => checkpoints.retain(|c| c.seq != seq),
            Err(_) => continue,
        }
    }
    let meta = meta.ok_or_else(|| format!("{} has no session metadata", path.display()))?;
    Ok(LoadedSession {
        meta,
        messages,
        checkpoints,
    })
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

        let mut resumed = Session::from_loaded(load_session(&found.path).unwrap());
        resumed.attach_recorder(store.reopen(&found));
        resumed.push_user("next");
        let loaded = load_session(&found.path).unwrap();
        assert_eq!(loaded.messages.len(), 3);
        assert_eq!(loaded.messages[0].content, "summary");
    }

    #[test]
    fn checkpoints_survive_reload_and_undo_is_recorded() {
        let tmp = TempDir::new().unwrap();
        let store = SessionStore::new(tmp.path().join("sessions"));
        let file = tmp.path().join("a.txt");
        std::fs::write(&file, "v1").unwrap();

        let mut session = Session::new();
        session.attach_recorder(store.create(tmp.path(), "openai", "test/model").unwrap());
        session.begin_checkpoint("first");
        session.push_user("first");
        session.capture_before_write(std::slice::from_ref(&file));
        std::fs::write(&file, "v2").unwrap();
        session.begin_checkpoint("second");
        session.push_user("second");
        session.capture_before_write(std::slice::from_ref(&file));
        std::fs::write(&file, "v3").unwrap();
        session.pop_checkpoint().unwrap().restore().unwrap();
        assert!(session.take_persist_error().is_none());

        let found = store.find("last", tmp.path()).unwrap();
        let mut reloaded = Session::from_loaded(load_session(&found.path).unwrap());
        let cp = reloaded
            .pop_checkpoint()
            .expect("first checkpoint persisted");
        assert_eq!(cp.label, "first");
        cp.restore().unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "v1");
    }

    #[test]
    fn write_errors_are_reported() {
        let tmp = TempDir::new().unwrap();
        let blocker = tmp.path().join("not-a-dir");
        std::fs::write(&blocker, "").unwrap();
        let store = SessionStore::new(tmp.path());
        let mut rec = store.create(tmp.path(), "openai", "test/model").unwrap();
        rec.path = blocker.join("x.jsonl");
        let mut session = Session::new();
        session.attach_recorder(rec);
        session.push_user("hi");
        assert!(session.take_persist_error().is_some());
        assert!(session.take_persist_error().is_none());
    }
}
