//! Per-turn file checkpoints so `/undo` can restore what the agent changed.

use std::path::{Path, PathBuf};

const MAX_SNAPSHOT_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_CHECKPOINTS: usize = 50;

#[derive(Debug, Clone, Default)]
pub struct Checkpoint {
    pub label: String,
    /// Original content of each touched file; `None` means it did not exist.
    files: Vec<(PathBuf, Option<Vec<u8>>)>,
}

impl Checkpoint {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            files: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(|(p, _)| p.as_path())
    }

    /// Records the current content of `path` unless it was already captured.
    pub fn capture(&mut self, path: &Path) {
        if self.files.iter().any(|(p, _)| p == path) {
            return;
        }
        let original = match std::fs::metadata(path) {
            Ok(meta) if meta.is_file() && meta.len() <= MAX_SNAPSHOT_BYTES => {
                std::fs::read(path).ok()
            }
            Ok(_) => return,
            Err(_) => None,
        };
        self.files.push((path.to_path_buf(), original));
    }

    /// Writes every file back to its captured state and returns the restored paths.
    pub fn restore(&self) -> std::io::Result<Vec<PathBuf>> {
        for (path, original) in self.files.iter().rev() {
            match original {
                Some(bytes) => {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::write(path, bytes)?;
                }
                None if path.exists() => std::fs::remove_file(path)?,
                None => {}
            }
        }
        Ok(self.files.iter().map(|(p, _)| p.clone()).collect())
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

        let mut cp = Checkpoint::new("turn");
        cp.capture(&existing);
        cp.capture(&created);
        std::fs::write(&existing, "v2").unwrap();
        cp.capture(&existing);
        std::fs::create_dir_all(created.parent().unwrap()).unwrap();
        std::fs::write(&created, "x").unwrap();

        assert_eq!(cp.restore().unwrap().len(), 2);
        assert_eq!(std::fs::read_to_string(&existing).unwrap(), "v1");
        assert!(!created.exists());
    }
}
