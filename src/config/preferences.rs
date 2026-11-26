use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const CURRENT_TERMS_VERSION: &str = "2026-11";
const MAX_RECENT_WORKSPACES: usize = 8;

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UserPreferences {
    #[serde(default)]
    pub terms_accepted: bool,
    #[serde(default)]
    pub terms_version: String,
    #[serde(default)]
    pub telemetry_enabled: bool,
    #[serde(default)]
    pub last_workspace: Option<PathBuf>,
    #[serde(default)]
    pub recent_workspaces: Vec<PathBuf>,
}

impl UserPreferences {
    pub fn default_path() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("ecliven-code").join("preferences.toml"))
    }

    pub fn load_default() -> Self {
        match Self::default_path() {
            Some(path) => Self::load_from(&path),
            None => Self::default(),
        }
    }

    pub fn load_from(path: &Path) -> Self {
        let Ok(raw) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        toml::from_str(&raw).unwrap_or_default()
    }

    pub fn save_default(&self) -> Result<(), std::io::Error> {
        if let Some(path) = Self::default_path() {
            self.save_to(&path)?;
        }
        Ok(())
    }

    pub fn save_to(&self, path: &Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let serialized = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, serialized)
    }

    pub fn needs_onboarding(&self) -> bool {
        !self.terms_accepted || self.terms_version != CURRENT_TERMS_VERSION
    }

    pub fn accept_terms(&mut self) {
        self.terms_accepted = true;
        self.terms_version = CURRENT_TERMS_VERSION.to_string();
    }

    pub fn record_workspace(&mut self, dir: PathBuf) {
        let canonical = dir.canonicalize().unwrap_or(dir);
        self.last_workspace = Some(canonical.clone());
        self.recent_workspaces.retain(|p| p != &canonical);
        self.recent_workspaces.insert(0, canonical);
        self.recent_workspaces.truncate(MAX_RECENT_WORKSPACES);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn saves_and_reloads_preferences_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("ecliven-code/preferences.toml");

        let mut prefs = UserPreferences::default();
        assert!(prefs.needs_onboarding());

        prefs.accept_terms();
        prefs.telemetry_enabled = true;
        prefs.record_workspace(tmp.path().to_path_buf());
        prefs.save_to(&path).unwrap();

        let reloaded = UserPreferences::load_from(&path);
        assert!(!reloaded.needs_onboarding());
        assert!(reloaded.terms_accepted);
        assert!(reloaded.telemetry_enabled);
        assert_eq!(reloaded.recent_workspaces.len(), 1);
    }
}
