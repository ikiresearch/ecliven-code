use std::time::Duration;

use reqwest::Client;
use serde_json::Value;
use tokio::process::Command;

use crate::ai::cli_backend::find_executable_in_path;
use crate::config::{AppConfig, ProviderKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
}

impl ModelOption {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

pub async fn fetch_models_for_config(config: &AppConfig) -> Vec<ModelOption> {
    let kind = config.active_provider;
    let settings = config.active_settings();

    let live = match kind {
        ProviderKind::AgyCli => fetch_agy_models(&settings.base_url).await,
        ProviderKind::OpenCodeCli => fetch_opencode_models(&settings.base_url).await,
        ProviderKind::Ollama => fetch_ollama_models(&settings.base_url).await,
        ProviderKind::OpenAi | ProviderKind::OpenRouter => {
            fetch_openai_compatible_models(&settings.base_url, settings.api_key.as_deref()).await
        }
        ProviderKind::Anthropic => {
            fetch_anthropic_models(&settings.base_url, settings.api_key.as_deref()).await
        }
        ProviderKind::CodexCli
        | ProviderKind::ClaudeCli
        | ProviderKind::GeminiCli
        | ProviderKind::CustomCli => Vec::new(),
    };

    let mut result = if live.is_empty() {
        curated_fallback_models(kind)
    } else {
        live
    };

    if kind.is_cli_backend() && !result.iter().any(|m| m.id == "default") {
        result.insert(0, ModelOption::new("default", "Default (CLI config)"));
    }

    let current_model = settings.model.trim();
    if !current_model.is_empty() && !result.iter().any(|m| m.id == current_model) {
        result.insert(
            0,
            ModelOption::new(current_model, format!("{current_model} (current)")),
        );
    }

    result
}

pub fn curated_fallback_models(kind: ProviderKind) -> Vec<ModelOption> {
    match kind {
        ProviderKind::Anthropic => vec![
            ModelOption::new("claude-sonnet-4-6", "Claude Sonnet 4.6"),
            ModelOption::new("claude-opus-4-6", "Claude Opus 4.6"),
            ModelOption::new("claude-sonnet-4-20250514", "Claude Sonnet 4"),
            ModelOption::new("claude-haiku-4-5-20251001", "Claude Haiku 4.5"),
        ],
        ProviderKind::OpenAi => vec![
            ModelOption::new("gpt-4.1", "GPT-4.1"),
            ModelOption::new("gpt-4.1-mini", "GPT-4.1 Mini"),
            ModelOption::new("o3", "o3"),
            ModelOption::new("o4-mini", "o4-mini"),
        ],
        ProviderKind::OpenRouter => vec![
            ModelOption::new("anthropic/claude-sonnet-4", "Claude Sonnet 4"),
            ModelOption::new("openai/gpt-4.1", "GPT-4.1"),
            ModelOption::new("deepseek/deepseek-r1", "DeepSeek R1"),
            ModelOption::new("google/gemini-2.5-pro", "Gemini 2.5 Pro"),
        ],
        ProviderKind::Ollama => vec![
            ModelOption::new("qwen2.5-coder:14b", "Qwen 2.5 Coder 14B"),
            ModelOption::new("qwen2.5-coder:7b", "Qwen 2.5 Coder 7B"),
            ModelOption::new("deepseek-coder-v2:16b", "DeepSeek Coder V2 16B"),
            ModelOption::new("llama3.2", "Llama 3.2"),
        ],
        ProviderKind::CodexCli => vec![
            ModelOption::new("default", "Default (Codex config)"),
            ModelOption::new("gpt-5.3-codex", "GPT-5.3 Codex"),
            ModelOption::new("gpt-5.2-codex", "GPT-5.2 Codex"),
            ModelOption::new("o3", "o3"),
            ModelOption::new("o4-mini", "o4-mini"),
            ModelOption::new("gpt-4.1", "GPT-4.1"),
        ],
        ProviderKind::AgyCli => vec![
            ModelOption::new("default", "Default (Agy config)"),
            ModelOption::new("claude-sonnet-4-6", "Claude Sonnet 4.6"),
            ModelOption::new("claude-opus-4-6-thinking", "Claude Opus 4.6 (Thinking)"),
            ModelOption::new("gemini-3.8-flash-high", "Gemini 3.8 Flash (High)"),
        ],
        ProviderKind::ClaudeCli => vec![
            ModelOption::new("default", "Default (Claude Code config)"),
            ModelOption::new("sonnet", "Claude Sonnet (alias)"),
            ModelOption::new("opus", "Claude Opus (alias)"),
            ModelOption::new("haiku", "Claude Haiku (alias)"),
            ModelOption::new("claude-sonnet-4-6", "Claude Sonnet 4.6"),
            ModelOption::new("claude-opus-4-6", "Claude Opus 4.6"),
        ],
        ProviderKind::OpenCodeCli => vec![
            ModelOption::new("default", "Default (OpenCode config)"),
            ModelOption::new("anthropic/claude-sonnet-4-6", "Claude Sonnet 4.6"),
            ModelOption::new("anthropic/claude-opus-4-6", "Claude Opus 4.6"),
        ],
        ProviderKind::GeminiCli => vec![
            ModelOption::new("default", "Default (Gemini CLI config)"),
            ModelOption::new("gemini-2.5-pro", "Gemini 2.5 Pro"),
            ModelOption::new("gemini-2.5-flash", "Gemini 2.5 Flash"),
        ],
        ProviderKind::CustomCli => vec![ModelOption::new("default", "Default (Custom CLI)")],
    }
}

async fn run_cli_capture(program: &str, args: &[&str]) -> Option<String> {
    let mut cmd = if cfg!(windows) {
        if let Some(found) = find_executable_in_path(program) {
            let ext = found
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext == "ps1" {
                let mut c = Command::new("powershell");
                c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
                c.arg(&found);
                c.args(args);
                c
            } else if ext == "cmd" || ext == "bat" {
                let mut c = Command::new("cmd");
                c.arg("/C").arg(&found).args(args);
                c
            } else {
                let mut c = Command::new(found);
                c.args(args);
                c
            }
        } else {
            let mut c = Command::new(program);
            c.args(args);
            c
        }
    } else {
        let mut c = Command::new(program);
        c.args(args);
        c
    };

    let output = tokio::time::timeout(Duration::from_secs(8), cmd.output())
        .await
        .ok()?
        .ok()?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    if stdout.trim().is_empty() {
        None
    } else {
        Some(stdout)
    }
}

async fn fetch_agy_models(binary: &str) -> Vec<ModelOption> {
    let bin = if binary.trim().is_empty() {
        "agy"
    } else {
        binary
    };
    let Some(stdout) = run_cli_capture(bin, &["models"]).await else {
        return Vec::new();
    };
    parse_agy_models(&stdout)
}

pub fn parse_agy_models(stdout: &str) -> Vec<ModelOption> {
    stdout
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("Fetching") {
                return None;
            }
            if let Some((id, label)) = trimmed.split_once('\t') {
                let id = id.trim();
                let label = label.trim();
                if !id.is_empty() {
                    return Some(ModelOption::new(
                        id,
                        if label.is_empty() { id } else { label },
                    ));
                }
            }
            let token = trimmed.split_whitespace().next()?;
            if token.contains('-') || token.contains('/') {
                Some(ModelOption::new(token, trimmed))
            } else {
                None
            }
        })
        .collect()
}

async fn fetch_opencode_models(binary: &str) -> Vec<ModelOption> {
    let bin = if binary.trim().is_empty() {
        "opencode"
    } else {
        binary
    };
    let Some(stdout) = run_cli_capture(bin, &["models"]).await else {
        return Vec::new();
    };
    parse_opencode_models(&stdout)
}

pub fn parse_opencode_models(stdout: &str) -> Vec<ModelOption> {
    stdout
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.contains(' ') || !trimmed.contains('/') {
                return None;
            }
            Some(ModelOption::new(trimmed, trimmed))
        })
        .collect()
}

async fn fetch_ollama_models(base_url: &str) -> Vec<ModelOption> {
    let Ok(client) = Client::builder().timeout(Duration::from_secs(3)).build() else {
        return Vec::new();
    };
    let url = format!("{}/api/tags", base_url.trim_end_matches('/'));
    let Ok(resp) = client.get(&url).send().await else {
        return Vec::new();
    };
    let Ok(json) = resp.json::<Value>().await else {
        return Vec::new();
    };
    json.get("models")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("name").and_then(Value::as_str))
                .map(|name| ModelOption::new(name, name))
                .collect()
        })
        .unwrap_or_default()
}

async fn fetch_openai_compatible_models(base_url: &str, api_key: Option<&str>) -> Vec<ModelOption> {
    let Some(key) = api_key else {
        return Vec::new();
    };
    let Ok(client) = Client::builder().timeout(Duration::from_secs(4)).build() else {
        return Vec::new();
    };
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let Ok(resp) = client.get(&url).bearer_auth(key).send().await else {
        return Vec::new();
    };
    let Ok(json) = resp.json::<Value>().await else {
        return Vec::new();
    };
    let mut models: Vec<ModelOption> = json
        .get("data")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m.get("id").and_then(Value::as_str)?;
                    let label = m
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(id);
                    Some(ModelOption::new(id, label))
                })
                .collect()
        })
        .unwrap_or_default();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models
}

async fn fetch_anthropic_models(base_url: &str, api_key: Option<&str>) -> Vec<ModelOption> {
    let Some(key) = api_key else {
        return Vec::new();
    };
    let Ok(client) = Client::builder().timeout(Duration::from_secs(4)).build() else {
        return Vec::new();
    };
    let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
    let Ok(resp) = client
        .get(&url)
        .header("x-api-key", key)
        .header("anthropic-version", "2023-06-01")
        .send()
        .await
    else {
        return Vec::new();
    };
    let Ok(json) = resp.json::<Value>().await else {
        return Vec::new();
    };
    json.get("data")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    let id = m.get("id").and_then(Value::as_str)?;
                    let label = m
                        .get("display_name")
                        .and_then(Value::as_str)
                        .unwrap_or(id);
                    Some(ModelOption::new(id, label))
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_agy_tab_separated_models() {
        let sample = "\
Fetching available models...
gemini-4-hydrogen-high\tGemini 4 Hydrogen (High)
claude-sonnet-4-6\tClaude Sonnet 4.6 (Thinking)
";
        let models = parse_agy_models(sample);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gemini-4-hydrogen-high");
        assert_eq!(models[0].label, "Gemini 4 Hydrogen (High)");
        assert_eq!(models[1].id, "claude-sonnet-4-6");
    }

    #[test]
    fn parses_opencode_models_list() {
        let sample = "\
anthropic/claude-sonnet-4-6
google/gemini-3.8-flash
some header line with spaces
";
        let models = parse_opencode_models(sample);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "anthropic/claude-sonnet-4-6");
        assert_eq!(models[1].id, "google/gemini-3.8-flash");
    }
}
