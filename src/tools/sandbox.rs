//! OS-level sandbox for `run_command`.
//! Linux: Landlock (re-exec of `ecliven __sandbox`). macOS: Seatbelt (`sandbox-exec`).
//! Other platforms run commands unsandboxed and say so in the output.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tokio::process::Command;

pub const SANDBOX_SUBCOMMAND: &str = "__sandbox";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxMode {
    /// Commands may read anything but write nowhere (except /dev and temp dirs).
    ReadOnly,
    /// Commands may write inside the workspace, temp dirs and extra writable roots.
    #[default]
    WorkspaceWrite,
    /// No sandbox.
    DangerFullAccess,
}

impl SandboxMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "read-only" | "readonly" | "ro" => Some(Self::ReadOnly),
            "workspace-write" | "workspace" | "on" => Some(Self::WorkspaceWrite),
            "danger-full-access" | "full" | "off" | "none" => Some(Self::DangerFullAccess),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxPolicy {
    pub mode: SandboxMode,
    pub network_access: bool,
    pub writable_roots: Vec<PathBuf>,
    /// Binary that implements `__sandbox` (defaults to the current executable).
    pub helper_exe: Option<PathBuf>,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self::unrestricted()
    }
}

impl SandboxPolicy {
    pub fn unrestricted() -> Self {
        Self {
            mode: SandboxMode::DangerFullAccess,
            network_access: true,
            writable_roots: Vec::new(),
            helper_exe: None,
        }
    }

    pub fn new(mode: SandboxMode, network_access: bool, writable_roots: Vec<PathBuf>) -> Self {
        Self {
            mode,
            network_access,
            writable_roots,
            helper_exe: None,
        }
    }

    pub fn is_restricted(&self) -> bool {
        self.mode != SandboxMode::DangerFullAccess || !self.network_access
    }

    pub fn describe(&self) -> String {
        let backend = if !self.is_restricted() {
            "none"
        } else if cfg!(target_os = "linux") {
            "landlock"
        } else if cfg!(target_os = "macos") {
            "seatbelt"
        } else {
            "unavailable on this OS"
        };
        format!(
            "{} (network: {}, backend: {backend})",
            self.mode.as_str(),
            if self.network_access { "on" } else { "off" }
        )
    }

    fn writable_paths(&self, workspace: &Path) -> Vec<PathBuf> {
        let mut paths = vec![PathBuf::from("/dev"), std::env::temp_dir()];
        if cfg!(unix) {
            paths.push(PathBuf::from("/tmp"));
        }
        if self.mode == SandboxMode::WorkspaceWrite {
            paths.push(workspace.to_path_buf());
            paths.extend(self.writable_roots.iter().cloned());
        }
        paths.into_iter().filter(|p| p.exists()).collect()
    }
}

fn shell_invocation(cmd: &str) -> (String, Vec<String>) {
    if cfg!(windows) {
        ("cmd".into(), vec!["/C".into(), cmd.into()])
    } else {
        ("sh".into(), vec!["-c".into(), cmd.into()])
    }
}

/// Builds the process for a shell command. The note is shown when the policy
/// asks for a sandbox that this platform cannot provide.
pub fn build_command(
    policy: &SandboxPolicy,
    workspace: &Path,
    cmd: &str,
) -> (Command, Option<String>) {
    let (program, args) = shell_invocation(cmd);
    let plain = || {
        let mut c = Command::new(&program);
        c.args(&args);
        c
    };

    if !policy.is_restricted() {
        return (plain(), None);
    }

    if cfg!(target_os = "linux") {
        let exe = policy
            .helper_exe
            .clone()
            .or_else(|| std::env::current_exe().ok());
        if let Some(exe) = exe {
            let mut c = Command::new(exe);
            c.arg(SANDBOX_SUBCOMMAND)
                .arg(policy.mode.as_str())
                .arg(if policy.network_access { "on" } else { "off" });
            for p in policy.writable_paths(workspace) {
                c.arg("--writable").arg(p);
            }
            c.arg("--").arg(&program).args(&args);
            return (c, None);
        }
    }

    if cfg!(target_os = "macos") {
        let mut c = Command::new("/usr/bin/sandbox-exec");
        c.arg("-p")
            .arg(seatbelt_profile(policy, workspace))
            .arg(&program)
            .args(&args);
        return (c, None);
    }

    (
        plain(),
        Some(format!(
            "[sandbox] {} is not available on this OS; command ran unsandboxed.",
            policy.mode.as_str()
        )),
    )
}

pub fn seatbelt_profile(policy: &SandboxPolicy, workspace: &Path) -> String {
    let quote = |p: &Path| {
        let raw = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        format!(
            "\"{}\"",
            raw.display()
                .to_string()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        )
    };
    let mut profile =
        String::from("(version 1)\n(allow default)\n(deny file-write*)\n(allow file-write*");
    for p in policy.writable_paths(workspace) {
        profile.push_str(&format!(" (subpath {})", quote(&p)));
    }
    profile.push_str(")\n");
    if !policy.network_access {
        profile
            .push_str("(deny network-outbound (remote ip))\n(deny network-inbound (local ip))\n");
    }
    profile
}

/// Entry point for `ecliven __sandbox <mode> <on|off> [--writable PATH]... -- PROGRAM [ARGS]...`.
/// Applies the sandbox to this process and then replaces it with PROGRAM.
pub fn sandbox_main(args: &[String]) -> ! {
    match run_sandbox_main(args) {
        Ok(never) => match never {},
        Err(err) => {
            eprintln!("[ecliven sandbox] {err}");
            std::process::exit(126);
        }
    }
}

enum Never {}

fn run_sandbox_main(args: &[String]) -> Result<Never, String> {
    let mode = args
        .first()
        .and_then(|m| SandboxMode::parse(m))
        .ok_or("missing or invalid sandbox mode")?;
    let network_access = args.get(1).map(String::as_str) != Some("off");
    let mut writable = Vec::new();
    let mut i = 2;
    while i < args.len() && args[i] != "--" {
        if args[i] == "--writable" {
            let path = args.get(i + 1).ok_or("--writable needs a path")?;
            writable.push(PathBuf::from(path));
            i += 2;
        } else {
            return Err(format!("unexpected argument '{}'", args[i]));
        }
    }
    let command = args
        .get(i + 1..)
        .filter(|c| !c.is_empty())
        .ok_or("missing command")?;
    apply_current_process_sandbox(mode, network_access, &writable)?;
    exec_replace(&command[0], &command[1..])
}

#[cfg(target_os = "linux")]
fn apply_current_process_sandbox(
    mode: SandboxMode,
    network_access: bool,
    writable: &[PathBuf],
) -> Result<(), String> {
    use landlock::{
        path_beneath_rules, Access, AccessFs, AccessNet, Ruleset, RulesetAttr, RulesetCreatedAttr,
        RulesetStatus, ABI,
    };

    if mode == SandboxMode::DangerFullAccess && network_access {
        return Ok(());
    }
    let abi = ABI::V5;
    let err = |e: landlock::RulesetError| e.to_string();
    let mut ruleset = Ruleset::default();
    if mode != SandboxMode::DangerFullAccess {
        ruleset = ruleset
            .handle_access(AccessFs::from_all(abi))
            .map_err(err)?;
    }
    if !network_access {
        ruleset = ruleset
            .handle_access(AccessNet::from_all(abi))
            .map_err(err)?;
    }
    let mut created = ruleset.create().map_err(err)?;
    if mode != SandboxMode::DangerFullAccess {
        created = created
            .add_rules(path_beneath_rules(["/"], AccessFs::from_read(abi)))
            .map_err(err)?
            .add_rules(path_beneath_rules(writable, AccessFs::from_all(abi)))
            .map_err(err)?;
    }
    let status = created.restrict_self().map_err(err)?;
    if status.ruleset == RulesetStatus::NotEnforced {
        eprintln!(
            "[ecliven sandbox] Landlock is not supported by this kernel; running unsandboxed."
        );
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn apply_current_process_sandbox(
    _mode: SandboxMode,
    _network_access: bool,
    _writable: &[PathBuf],
) -> Result<(), String> {
    Err("the __sandbox helper is only available on Linux".into())
}

#[cfg(unix)]
fn exec_replace(program: &str, args: &[String]) -> Result<Never, String> {
    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new(program).args(args).exec();
    Err(format!("failed to exec '{program}': {err}"))
}

#[cfg(not(unix))]
fn exec_replace(program: &str, args: &[String]) -> Result<Never, String> {
    let status = std::process::Command::new(program)
        .args(args)
        .status()
        .map_err(|e| format!("failed to run '{program}': {e}"))?;
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modes_and_defaults_to_unrestricted_context() {
        assert_eq!(SandboxMode::parse("read-only"), Some(SandboxMode::ReadOnly));
        assert_eq!(
            SandboxMode::parse("off"),
            Some(SandboxMode::DangerFullAccess)
        );
        assert_eq!(SandboxMode::parse("bogus"), None);
        assert!(!SandboxPolicy::default().is_restricted());
        assert!(SandboxPolicy::new(SandboxMode::DangerFullAccess, false, vec![]).is_restricted());
    }

    #[cfg(unix)]
    #[test]
    fn seatbelt_profile_limits_writes_and_network() {
        let tmp = tempfile::TempDir::new().unwrap();
        let policy = SandboxPolicy::new(SandboxMode::WorkspaceWrite, false, vec![]);
        let profile = seatbelt_profile(&policy, tmp.path());
        assert!(profile.contains("(deny file-write*)"));
        assert!(profile.contains(&tmp.path().canonicalize().unwrap().display().to_string()));
        assert!(profile.contains("deny network-outbound"));
    }
}
