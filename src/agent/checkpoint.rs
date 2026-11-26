//! Per-turn file checkpoints so `/undo` can restore what the agent changed.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const MAX_SNAPSHOT_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_CHECKPOINTS: usize = 50;

/// State of a file before the agent touched it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FileOriginal {
    /// The file did not exist; undo deletes it.
    Missing,
    /// Original bytes (base64 in session files); undo writes them back.
    Content {
        #[serde(with = "b64")]
        bytes: Vec<u8>,
    },
    /// Could not be captured (unreadable or too large); undo leaves it alone.
    Unavailable { reason: String },
}

mod b64 {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let raw = String::deserialize(d)?;
        base64::engine::general_purpose::STANDARD
            .decode(raw)
            .map_err(serde::de::Error::custom)
    }
}

pub fn capture_original(path: &Path) -> FileOriginal {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileOriginal::Missing,
        Err(e) => FileOriginal::Unavailable {
            reason: e.to_string(),
        },
        Ok(meta) if !meta.is_file() => FileOriginal::Unavailable {
            reason: "not a regular file".into(),
        },
        Ok(meta) if meta.len() > MAX_SNAPSHOT_BYTES => FileOriginal::Unavailable {
            reason: format!("larger than {} MiB", MAX_SNAPSHOT_BYTES / 1024 / 1024),
        },
        Ok(_) => match std::fs::read(path) {
            Ok(bytes) => FileOriginal::Content { bytes },
            Err(e) => FileOriginal::Unavailable {
                reason: e.to_string(),
            },
        },
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RestoreReport {
    pub restored: Vec<PathBuf>,
    pub skipped: Vec<(PathBuf, String)>,
}

#[derive(Debug, Clone, Default)]
pub struct Checkpoint {
    pub seq: u64,
    pub label: String,
    files: Vec<(PathBuf, FileOriginal)>,
}

impl Checkpoint {
    pub fn new(seq: u64, label: impl Into<String>) -> Self {
        Self {
            seq,
            label: label.into(),
            files: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.files.iter().any(|(p, _)| p == path)
    }

    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(|(p, _)| p.as_path())
    }

    /// Records the current state of `path` unless it was already captured.
    /// Returns the captured state when it is new.
    pub fn capture(&mut self, path: &Path) -> Option<&FileOriginal> {
        if self.contains(path) {
            return None;
        }
        self.insert(path.to_path_buf(), capture_original(path));
        self.files.last().map(|(_, o)| o)
    }

    pub fn insert(&mut self, path: PathBuf, original: FileOriginal) {
        if !self.contains(&path) {
            self.files.push((path, original));
        }
    }

    /// Writes every captured file back; unavailable snapshots are reported, never deleted.
    pub fn restore(&self) -> std::io::Result<RestoreReport> {
        let mut report = RestoreReport::default();
        for (path, original) in self.files.iter().rev() {
            match original {
                FileOriginal::Content { bytes } => {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(path, bytes)?;
                }
                FileOriginal::Missing => {
                    if path.exists() {
                        std::fs::remove_file(path)?;
                    }
                }
                FileOriginal::Unavailable { reason } => {
                    report.skipped.push((path.clone(), reason.clone()));
                    continue;
                }
            }
            report.restored.push(path.clone());
        }
        report.restored.reverse();
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn restores_modified_and_removes_created_files() {
        let tmp = TempDir::new().unwrap();
        let existing = tmp.path().join("a.txt");
        let created = tmp.path().join("new/b.txt");
        std::fs::write(&existing, "v1").unwrap();

        let mut cp = Checkpoint::new(1, "turn");
        assert!(cp.capture(&existing).is_some());
        cp.capture(&created);
        std::fs::write(&existing, "v2").unwrap();
        assert!(cp.capture(&existing).is_none());
        std::fs::create_dir_all(created.parent().unwrap()).unwrap();
        std::fs::write(&created, "x").unwrap();

        assert_eq!(cp.restore().unwrap().restored.len(), 2);
        assert_eq!(std::fs::read_to_string(&existing).unwrap(), "v1");
        assert!(!created.exists());
    }

    #[test]
    fn unreadable_file_is_skipped_not_deleted() {
        let tmp = TempDir::new().unwrap();
        let dir_path = tmp.path().join("dir");
        std::fs::create_dir(&dir_path).unwrap();

        let mut cp = Checkpoint::new(1, "turn");
        assert!(matches!(
            cp.capture(&dir_path),
            Some(FileOriginal::Unavailable { .. })
        ));
        let report = cp.restore().unwrap();
        assert!(report.restored.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert!(dir_path.exists());
    }

    #[test]
    fn original_round_trips_through_json() {
        let original = FileOriginal::Content {
            bytes: vec![0, 159, 146, 150],
        };
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(serde_json::from_str::<FileOriginal>(&json).unwrap(), original);
    }
}
