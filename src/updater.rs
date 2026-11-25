use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;
use thiserror::Error;
use tokio::process::Command;

pub const DEFAULT_REPO: &str = "ikiresearch/ecliven-code";
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("GitHub API returned HTTP {0}")]
    ApiStatus(u16),

    #[error("Failed to execute updater command: {0}")]
    Io(#[from] std::io::Error),

    #[error("Updater script failed (exit {code}): {output}")]
    InstallFailed { code: i32, output: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub version: String,
    pub tag_name: String,
    pub html_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    UpToDate { current: String },
    Available { current: String, latest: ReleaseInfo },
}

#[derive(Debug, Deserialize)]
struct GithubReleaseResponse {
    tag_name: String,
    html_url: String,
}

pub fn cleanup_stale_backup() {
    if let Ok(exe_path) = std::env::current_exe() {
        let old_path = exe_path.with_extension("exe.old");
        if old_path.exists() {
            let _ = std::fs::remove_file(old_path);
        }
    }
}

pub fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let cleaned = raw.trim().trim_start_matches(['v', 'V']);
    let core = cleaned.split('-').next().unwrap_or(cleaned);
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

pub fn is_newer_version(current: &str, candidate: &str) -> bool {
    match (parse_semver(current), parse_semver(candidate)) {
        (Some(curr), Some(cand)) => cand > curr,
        _ => false,
    }
}

pub async fn check_for_update() -> Result<UpdateStatus, UpdateError> {
    check_for_update_for_repo(DEFAULT_REPO, CURRENT_VERSION).await
}

pub async fn check_for_update_for_repo(
    repo: &str,
    current_version: &str,
) -> Result<UpdateStatus, UpdateError> {
    let client = Client::builder()
        .timeout(Duration::from_secs(4))
        .user_agent(format!("ecliven-code/{current_version}"))
        .build()?;

    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let resp = client.get(&url).send().await?;

    if !resp.status().is_success() {
        return Err(UpdateError::ApiStatus(resp.status().as_u16()));
    }

    let release: GithubReleaseResponse = resp.json().await?;
    let latest_ver = release
        .tag_name
        .trim()
        .trim_start_matches(['v', 'V'])
        .to_string();

    if is_newer_version(current_version, &latest_ver) {
        Ok(UpdateStatus::Available {
            current: current_version.to_string(),
            latest: ReleaseInfo {
                version: latest_ver,
                tag_name: release.tag_name,
                html_url: release.html_url,
            },
        })
    } else {
        Ok(UpdateStatus::UpToDate {
            current: current_version.to_string(),
        })
    }
}

pub async fn run_self_update(target_version: Option<&str>) -> Result<String, UpdateError> {
    let mut cmd = if cfg!(windows) {
        let ps_script = match target_version {
            Some(ver) => format!(
                "$env:ECLIVEN_VERSION = '{}'; irm https://github.com/{}/releases/latest/download/install.ps1 | iex",
                sanitize_version_arg(ver),
                DEFAULT_REPO
            ),
            None => format!(
                "irm https://github.com/{}/releases/latest/download/install.ps1 | iex",
                DEFAULT_REPO
            ),
        };
        let mut c = Command::new("powershell");
        c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &ps_script]);
        c
    } else {
        let sh_script = match target_version {
            Some(ver) => format!(
                "ECLIVEN_VERSION='{}' curl -fsSL https://github.com/{}/releases/latest/download/install.sh | sh",
                sanitize_version_arg(ver),
                DEFAULT_REPO
            ),
            None => format!(
                "curl -fsSL https://github.com/{}/releases/latest/download/install.sh | sh",
                DEFAULT_REPO
            ),
        };
        let mut c = Command::new("sh");
        c.args(["-c", &sh_script]);
        c
    };

    let output = cmd.output().await?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if output.status.success() {
        Ok(if stdout.is_empty() {
            "Ecliven Code updated successfully.".to_string()
        } else {
            stdout
        })
    } else {
        let combined = if stderr.is_empty() {
            stdout
        } else {
            format!("{stdout}\n{stderr}").trim().to_string()
        };
        Err(UpdateError::InstallFailed {
            code: output.status.code().unwrap_or(-1),
            output: combined,
        })
    }
}

fn sanitize_version_arg(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_semver_versions_accurately() {
        assert!(is_newer_version("0.1.0", "0.2.0"));
        assert!(is_newer_version("0.1.0", "v0.1.1"));
        assert!(is_newer_version("0.9.9", "1.0.0"));
        assert!(!is_newer_version("0.2.0", "0.2.0"));
        assert!(!is_newer_version("0.2.0", "0.1.9"));
        assert!(!is_newer_version("0.2.0", "not-a-version"));
    }

    #[test]
    fn sanitizes_version_argument_against_injection() {
        assert_eq!(sanitize_version_arg("v0.2.0; whoami"), "v0.2.0whoami");
        assert_eq!(sanitize_version_arg("v1.2.3-rc1"), "v1.2.3-rc1");
    }
}
