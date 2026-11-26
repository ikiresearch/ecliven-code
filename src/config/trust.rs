//! Workspace trust: settings in a repository's `.ecliven.toml` that can run code,
//! weaken approvals or redirect credentials are withheld until the user trusts
//! that workspace. Trust is bound to a fingerprint of those settings, so a later
//! change (for example a pulled commit adding an MCP server) asks again.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrustStatus {
    /// The project config has no executable settings.
    #[default]
    NotRequired,
    Trusted,
    /// Trusted for this run only (`--trust-workspace`).
    Session,
    /// Never decided for this workspace.
    Unknown,
    /// Trusted or denied before, but the executable settings changed since.
    Changed,
    Denied,
}

#[derive(Debug, Clone, Default)]
pub struct WorkspaceTrust {
    pub status: TrustStatus,
    pub key: String,
    /// Human-readable executable settings found in the project config.
    pub restricted: Vec<String>,
    pub fingerprint: String,
}

impl WorkspaceTrust {
    pub fn needs_decision(&self) -> bool {
        matches!(self.status, TrustStatus::Unknown | TrustStatus::Changed)
    }

    pub fn withheld(&self) -> bool {
        matches!(
            self.status,
            TrustStatus::Unknown | TrustStatus::Changed | TrustStatus::Denied
        )
    }

    pub fn describe(&self) -> String {
        let state = match self.status {
            TrustStatus::NotRequired => "no executable project settings",
            TrustStatus::Trusted => "trusted",
            TrustStatus::Session => "trusted for this run",
            TrustStatus::Unknown => "not trusted yet: executable project settings are disabled",
            TrustStatus::Changed => {
                "executable project settings changed since your last decision: they are disabled"
            }
            TrustStatus::Denied => "denied: executable project settings are disabled",
        };
        let mut out = format!("Workspace {}: {state}.", self.key);
        for item in &self.restricted {
            out.push_str(&format!("\n  - {item}"));
        }
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    trusted: bool,
    fingerprint: String,
    decided_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustStore {
    #[serde(skip)]
    path: Option<PathBuf>,
    #[serde(default)]
    workspaces: BTreeMap<String, Entry>,
}

/// Canonical path used as the trust key.
pub fn workspace_key(workspace: &Path) -> String {
    std::fs::canonicalize(workspace)
        .unwrap_or_else(|_| workspace.to_path_buf())
        .display()
        .to_string()
}

impl TrustStore {
    /// `ECLIVEN_TRUST_FILE`, or `<config dir>/ecliven-code/trusted_workspaces.json`.
    pub fn default_path() -> Option<PathBuf> {
        if let Some(p) = std::env::var_os("ECLIVEN_TRUST_FILE") {
            return Some(PathBuf::from(p));
        }
        dirs::config_dir().map(|d| d.join("ecliven-code").join("trusted_workspaces.json"))
    }

    pub fn load_default() -> Self {
        match Self::default_path() {
            Some(p) => Self::load(p),
            None => Self::default(),
        }
    }

    /// A missing or unreadable file means nothing is trusted.
    pub fn load(path: PathBuf) -> Self {
        let mut store: Self = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        store.path = Some(path);
        store
    }

    pub fn status(&self, key: &str, fingerprint: &str) -> TrustStatus {
        match self.workspaces.get(key) {
            None => TrustStatus::Unknown,
            Some(e) if e.fingerprint != fingerprint => TrustStatus::Changed,
            Some(e) if e.trusted => TrustStatus::Trusted,
            Some(_) => TrustStatus::Denied,
        }
    }

    pub fn set(&mut self, key: &str, trusted: bool, fingerprint: &str) {
        let decided_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.workspaces.insert(
            key.to_string(),
            Entry {
                trusted,
                fingerprint: fingerprint.to_string(),
                decided_at,
            },
        );
    }

    pub fn revoke(&mut self, key: &str) -> bool {
        self.workspaces.remove(key).is_some()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = self.path.as_ref().ok_or("no trust file location")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let tmp = path.with_extension("json.tmp");
        let raw = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, raw).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    /// Records a decision for `trust` and saves it.
    pub fn decide(trust: &WorkspaceTrust, trusted: bool) -> Result<(), String> {
        let mut store = Self::load_default();
        store.set(&trust.key, trusted, &trust.fingerprint);
        store.save()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tempfile::TempDir;

    use super::*;
    use crate::config::{AppConfig, ApprovalMode, CliOverrides};
    use crate::tools::sandbox::SandboxMode;

    const RISKY: &str = r#"
model = "test/model"
approval_mode = "auto-all"
sandbox_mode = "danger-full-access"
sandbox_writable_roots = ["/tmp/example"]

[providers.openai]
base_url = "https://example.com/v1"

[mcp_servers.tools]
command = "npx"
args = ["-y", "@example/server"]
env = { TOKEN = "secret" }
"#;

    fn load(ws: &Path, store: &TrustStore, overrides: &CliOverrides) -> AppConfig {
        AppConfig::load_from_sources_with_trust(
            ws.to_path_buf(),
            None,
            Some(&ws.join(".ecliven.toml")),
            &HashMap::new(),
            overrides,
            store,
        )
        .unwrap()
    }

    fn workspace(config: &str) -> TempDir {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join(".ecliven.toml"), config).unwrap();
        tmp
    }

    #[test]
    fn unknown_workspace_keeps_safe_settings_and_withholds_executable_ones() {
        let ws = workspace(RISKY);
        let cfg = load(ws.path(), &TrustStore::default(), &CliOverrides::default());
        assert_eq!(cfg.workspace_trust.status, TrustStatus::Unknown);
        assert!(cfg.workspace_trust.needs_decision());
        assert_eq!(cfg.workspace_trust.restricted.len(), 5, "{:?}", cfg.workspace_trust.restricted);
        assert!(cfg.workspace_trust.describe().contains("MCP server 'tools' starts `npx -y @example/server` with env TOKEN"));
        assert!(!cfg.workspace_trust.describe().contains("secret"));
        assert_eq!(cfg.approval_mode, ApprovalMode::Ask);
        assert_eq!(cfg.sandbox_mode, SandboxMode::WorkspaceWrite);
        assert!(cfg.sandbox_writable_roots.is_empty() && cfg.mcp_servers.is_empty());
        assert_ne!(cfg.providers[&crate::config::ProviderKind::OpenAi].base_url, "https://example.com/v1");
        assert_eq!(cfg.active_settings().model, "test/model");
    }

    #[test]
    fn trust_applies_until_executable_settings_change() {
        let ws = workspace(RISKY);
        let dir = TempDir::new().unwrap();
        let mut store = TrustStore::load(dir.path().join("trust.json"));
        let first = load(ws.path(), &store, &CliOverrides::default());
        store.set(&first.workspace_trust.key, true, &first.workspace_trust.fingerprint);
        store.save().unwrap();

        let store = TrustStore::load(dir.path().join("trust.json"));
        let cfg = load(ws.path(), &store, &CliOverrides::default());
        assert_eq!(cfg.workspace_trust.status, TrustStatus::Trusted);
        assert_eq!(cfg.approval_mode, ApprovalMode::AutoAll);
        assert_eq!(cfg.mcp_servers.len(), 1);

        std::fs::write(ws.path().join(".ecliven.toml"), RISKY.replace("secret", "other")).unwrap();
        let cfg = load(ws.path(), &store, &CliOverrides::default());
        assert_eq!(cfg.workspace_trust.status, TrustStatus::Changed);
        assert!(cfg.mcp_servers.is_empty());
    }

    #[test]
    fn denied_session_and_safe_configs() {
        let ws = workspace(RISKY);
        let mut store = TrustStore::default();
        let cfg = load(ws.path(), &store, &CliOverrides::default());
        store.set(&cfg.workspace_trust.key, false, &cfg.workspace_trust.fingerprint);
        let denied = load(ws.path(), &store, &CliOverrides::default());
        assert_eq!(denied.workspace_trust.status, TrustStatus::Denied);
        assert!(denied.workspace_trust.withheld() && !denied.workspace_trust.needs_decision());

        let session = CliOverrides {
            trust_workspace: true,
            ..CliOverrides::default()
        };
        let cfg = load(ws.path(), &store, &session);
        assert_eq!(cfg.workspace_trust.status, TrustStatus::Session);
        assert_eq!(cfg.sandbox_mode, SandboxMode::DangerFullAccess);

        let safe = workspace("approval_mode = \"ask\"\nsandbox_mode = \"read-only\"\nsaver_mode = true\n");
        let cfg = load(safe.path(), &TrustStore::default(), &CliOverrides::default());
        assert_eq!(cfg.workspace_trust.status, TrustStatus::NotRequired);
        assert_eq!(cfg.sandbox_mode, SandboxMode::ReadOnly);
        assert!(cfg.saver_mode);
    }
}
