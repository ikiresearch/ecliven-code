use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnboardingStep {
    Terms,
    Telemetry,
    Workspace,
}

#[derive(Debug, Clone)]
pub struct OnboardingState {
    pub step: OnboardingStep,
    pub telemetry_choice: bool,
    pub browse_dir: PathBuf,
    pub subdirs: Vec<PathBuf>,
    pub selected_index: usize,
    pub manual_path_mode: bool,
    pub manual_path_input: String,
    pub error_msg: Option<String>,
    pub allow_cancel: bool,
}

impl OnboardingState {
    pub fn new_full(initial_dir: PathBuf, initial_telemetry: bool, allow_cancel: bool) -> Self {
        let browse_dir = initial_dir.canonicalize().unwrap_or(initial_dir);
        let subdirs = list_subdirectories(&browse_dir);
        Self {
            step: OnboardingStep::Terms,
            telemetry_choice: initial_telemetry,
            manual_path_input: browse_dir.display().to_string(),
            browse_dir,
            subdirs,
            selected_index: 0,
            manual_path_mode: false,
            error_msg: None,
            allow_cancel,
        }
    }

    pub fn new_folder_picker(initial_dir: PathBuf, initial_telemetry: bool) -> Self {
        let mut st = Self::new_full(initial_dir, initial_telemetry, true);
        st.step = OnboardingStep::Workspace;
        st
    }

    pub fn total_workspace_items(&self) -> usize {
        // 0 = use current folder, 1 = parent (..), 2.. = child subdirs
        2 + self.subdirs.len()
    }

    pub fn move_up(&mut self) {
        self.error_msg = None;
        match self.step {
            OnboardingStep::Telemetry => {
                self.telemetry_choice = !self.telemetry_choice;
            }
            OnboardingStep::Workspace if !self.manual_path_mode && self.selected_index > 0 => {
                self.selected_index -= 1;
            }
            _ => {}
        }
    }

    pub fn move_down(&mut self) {
        self.error_msg = None;
        match self.step {
            OnboardingStep::Telemetry => {
                self.telemetry_choice = !self.telemetry_choice;
            }
            OnboardingStep::Workspace if !self.manual_path_mode => {
                let max_idx = self.total_workspace_items().saturating_sub(1);
                if self.selected_index < max_idx {
                    self.selected_index += 1;
                }
            }
            _ => {}
        }
    }

    pub fn enter_directory(&mut self, dir: PathBuf) {
        let canonical = dir.canonicalize().unwrap_or(dir);
        self.subdirs = list_subdirectories(&canonical);
        self.manual_path_input = canonical.display().to_string();
        self.browse_dir = canonical;
        self.selected_index = 0;
        self.error_msg = None;
    }

    pub fn go_parent(&mut self) {
        if let Some(parent) = self.browse_dir.parent() {
            self.enter_directory(parent.to_path_buf());
        }
    }

    pub fn open_selected_subdir(&mut self) {
        if self.selected_index == 1 {
            self.go_parent();
        } else if self.selected_index >= 2 {
            if let Some(dir) = self.subdirs.get(self.selected_index - 2).cloned() {
                self.enter_directory(dir);
            }
        }
    }

    pub fn toggle_manual_path(&mut self) {
        self.manual_path_mode = !self.manual_path_mode;
        self.error_msg = None;
        if self.manual_path_mode {
            self.manual_path_input = self.browse_dir.display().to_string();
        }
    }

    pub fn confirm_workspace(&mut self) -> Result<PathBuf, String> {
        if self.manual_path_mode {
            let trimmed = self.manual_path_input.trim();
            if trimmed.is_empty() {
                return Err("Ingresa una ruta de carpeta valida.".into());
            }
            let path = PathBuf::from(trimmed);
            if !path.exists() {
                return Err(format!("La carpeta '{}' no existe.", path.display()));
            }
            if !path.is_dir() {
                return Err(format!("'{}' no es una carpeta.", path.display()));
            }
            return Ok(path.canonicalize().unwrap_or(path));
        }

        match self.selected_index {
            0 => Ok(self.browse_dir.clone()),
            1 => {
                self.go_parent();
                Err(String::new())
            }
            idx => {
                if let Some(dir) = self.subdirs.get(idx - 2).cloned() {
                    self.enter_directory(dir);
                    Err(String::new())
                } else {
                    Ok(self.browse_dir.clone())
                }
            }
        }
    }
}

fn list_subdirectories(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if !path.is_dir() {
                return None;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || matches!(name.as_str(), "target" | "node_modules") {
                return None;
            }
            Some(path)
        })
        .collect();
    dirs.sort_by(|a, b| {
        let an = a
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let bn = b
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        an.cmp(&bn)
    });
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn folder_picker_browses_subdirectories_and_confirms() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("alpha")).unwrap();
        std::fs::create_dir_all(tmp.path().join("beta")).unwrap();

        let mut ob = OnboardingState::new_folder_picker(tmp.path().to_path_buf(), false);
        assert_eq!(ob.subdirs.len(), 2);

        // Move down to index 2 ('alpha') and open it
        ob.move_down();
        ob.move_down();
        ob.open_selected_subdir();
        assert!(ob.browse_dir.ends_with("alpha"));

        // Confirm current folder (index 0)
        let selected = ob.confirm_workspace().unwrap();
        assert!(selected.ends_with("alpha"));
    }

    #[test]
    fn manual_path_mode_validates_nonexistent_directory() {
        let tmp = TempDir::new().unwrap();
        let mut ob = OnboardingState::new_folder_picker(tmp.path().to_path_buf(), false);
        ob.toggle_manual_path();
        ob.manual_path_input = tmp.path().join("does-not-exist").display().to_string();

        let err = ob.confirm_workspace().unwrap_err();
        assert!(err.contains("no existe"));
    }
}
