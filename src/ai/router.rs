//! Model selection across providers: `provider/model` references, a
//! heuristic router (difficulty, cost tier, specialty) and saver mode.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::Deserialize;

use crate::ai::registry::build_provider;
use crate::ai::{AiError, LlmProvider};
use crate::config::{AppConfig, ProviderKind};

/// `provider/model`, e.g. `anthropic/claude-sonnet-4-5` or `openrouter/deepseek/deepseek-chat`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRef {
    pub provider: ProviderKind,
    pub model: String,
}

impl ModelRef {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        let (provider, model) = raw.split_once('/').unwrap_or((raw, ""));
        let provider = ProviderKind::parse(provider).map_err(|e| e.to_string())?;
        let model = if model.trim().is_empty() {
            provider.default_model().to_string()
        } else {
            model.trim().to_string()
        };
        Ok(Self { provider, model })
    }

    pub fn label(&self) -> String {
        format!("{}/{}", self.provider.as_str(), self.model)
    }
}

/// Builds a provider for `model_ref`, reusing the API keys and URLs in `config`.
pub fn build_provider_for(
    config: &AppConfig,
    model_ref: &ModelRef,
) -> Result<Arc<dyn LlmProvider>, AiError> {
    let mut cfg = config.clone();
    cfg.active_provider = model_ref.provider;
    cfg.active_settings_mut().model = model_ref.model.clone();
    build_provider(&cfg)
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct RouterConfig {
    #[serde(default)]
    pub enabled: bool,
    pub cheap: Option<String>,
    pub standard: Option<String>,
    pub strong: Option<String>,
    /// Specialty (security, frontend, backend, data, tests, docs, devops) -> model.
    #[serde(default)]
    pub specialties: BTreeMap<String, String>,
    /// Learn from real calls and skip unhealthy models (default true).
    pub adaptive: Option<bool>,
    /// Tried in order after the routed model on 429, outages or context overflow.
    #[serde(default)]
    pub fallbacks: Vec<String>,
    pub max_session_usd: Option<f64>,
    pub max_daily_usd: Option<f64>,
    /// Models whose average latency exceeds this are skipped when others exist.
    pub max_latency_ms: Option<u64>,
    /// `provider/model` -> USD per million input/output tokens, for budgets.
    #[serde(default)]
    pub prices: BTreeMap<String, crate::ai::adaptive::Price>,
}

impl RouterConfig {
    pub fn is_adaptive(&self) -> bool {
        self.adaptive.unwrap_or(true)
    }

    pub fn budget(&self) -> crate::ai::adaptive::Budget {
        crate::ai::adaptive::Budget {
            max_session_usd: self.max_session_usd,
            max_daily_usd: self.max_daily_usd,
        }
    }

    fn usable(&self, raw: &str) -> bool {
        !self.is_adaptive()
            || ModelRef::parse(raw).is_ok_and(|m| {
                crate::ai::adaptive::StatsStore::shared().healthy(&m.label(), self.max_latency_ms)
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Difficulty {
    Simple,
    Standard,
    Complex,
}

impl Difficulty {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Simple => "simple",
            Self::Standard => "standard",
            Self::Complex => "complex",
        }
    }

    fn cheaper(self) -> Self {
        match self {
            Self::Complex => Self::Standard,
            _ => Self::Simple,
        }
    }
}

const SPECIALTIES: &[(&str, &[&str])] = &[
    (
        "security",
        &[
            "security",
            "vulnerab",
            "cve",
            "xss",
            "csrf",
            "injection",
            "auth",
            "seguridad",
            "exploit",
            "owasp",
        ],
    ),
    (
        "frontend",
        &[
            "css",
            "html",
            "react",
            "vue",
            "svelte",
            "tailwind",
            "frontend",
            "ui ",
            "componente",
            "component",
        ],
    ),
    (
        "data",
        &[
            "sql",
            "database",
            "query",
            "migration",
            "postgres",
            "mysql",
            "sqlite",
            "base de datos",
        ],
    ),
    ("tests", &["test", "prueba", "coverage", "flaky", "assert"]),
    (
        "docs",
        &[
            "readme",
            "docs",
            "document",
            "changelog",
            "comment",
            "comentario",
        ],
    ),
    (
        "devops",
        &[
            "docker",
            "kubernetes",
            "ci ",
            "pipeline",
            "deploy",
            "terraform",
            "github actions",
            "workflow",
        ],
    ),
    (
        "backend",
        &[
            "api", "endpoint", "server", "backend", "rust", "handler", "servidor",
        ],
    ),
];

const COMPLEX_HINTS: &[&str] = &[
    "refactor",
    "architecture",
    "arquitectura",
    "redesign",
    "rediseñ",
    "migrate",
    "migra",
    "concurrency",
    "race condition",
    "deadlock",
    "performance",
    "rendimiento",
    "optimiz",
    "implement",
    "implementa",
    "design",
    "diseña",
    "debug",
    "investigate",
    "investiga",
    "multiple files",
    "varios archivos",
    "whole project",
    "todo el proyecto",
    "security audit",
];

const SIMPLE_HINTS: &[&str] = &[
    "typo",
    "rename",
    "renombra",
    "explain",
    "explica",
    "what is",
    "qué es",
    "que es",
    "format",
    "formatea",
    "comment",
    "comenta",
    "list ",
    "lista ",
    "show",
    "muestra",
    "translate",
    "traduce",
    "summarize",
    "resume ",
];

/// Heuristic, zero-cost classification of a prompt.
pub fn classify(prompt: &str) -> (Difficulty, Option<&'static str>) {
    let lower = prompt.to_lowercase();
    let words = lower.split_whitespace().count();
    let complex = COMPLEX_HINTS.iter().filter(|h| lower.contains(*h)).count();
    let simple = SIMPLE_HINTS.iter().filter(|h| lower.contains(*h)).count();
    let code_blocks = lower.matches("```").count() / 2;

    let difficulty =
        if complex >= 2 || words > 250 || (complex >= 1 && words > 60) || code_blocks >= 2 {
            Difficulty::Complex
        } else if complex == 0 && (simple >= 1 || words <= 12) {
            Difficulty::Simple
        } else {
            Difficulty::Standard
        };

    let specialty = SPECIALTIES
        .iter()
        .map(|(name, keys)| (*name, keys.iter().filter(|k| lower.contains(*k)).count()))
        .filter(|(_, hits)| *hits > 0)
        .max_by_key(|(_, hits)| *hits)
        .map(|(name, _)| name);
    (difficulty, specialty)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDecision {
    pub model: ModelRef,
    pub difficulty: Difficulty,
    pub specialty: Option<&'static str>,
    pub reason: String,
}

fn tier_model(router: &RouterConfig, tier: Difficulty) -> Option<&String> {
    let order: &[Option<&String>] = match tier {
        Difficulty::Simple => &[
            router.cheap.as_ref(),
            router.standard.as_ref(),
            router.strong.as_ref(),
        ],
        Difficulty::Standard => &[
            router.standard.as_ref(),
            router.strong.as_ref(),
            router.cheap.as_ref(),
        ],
        Difficulty::Complex => &[
            router.strong.as_ref(),
            router.standard.as_ref(),
            router.cheap.as_ref(),
        ],
    };
    let all: Vec<&String> = order.iter().flatten().copied().collect();
    all.iter().find(|m| router.usable(m)).or(all.first()).copied()
}

/// Picks a model for `prompt`. Returns `None` when neither the router nor saver
/// mode applies, or no suitable model is configured (the active model is used).
pub fn route(config: &AppConfig, prompt: &str) -> Option<RouteDecision> {
    let router = &config.router;
    if !router.enabled && !config.saver_mode {
        return None;
    }
    let (difficulty, specialty) = classify(prompt);
    let tight = router.budget().nearly_exhausted(&crate::ai::adaptive::StatsStore::shared());
    let tier = if config.saver_mode || tight {
        difficulty.cheaper()
    } else {
        difficulty
    };

    let (raw, why) = if config.saver_mode && !router.enabled {
        (
            router.cheap.as_ref()?,
            "saver mode: cheapest model".to_string(),
        )
    } else if let Some(m) = specialty
        .filter(|_| tier != Difficulty::Simple)
        .and_then(|s| router.specialties.get(s))
        .filter(|m| router.usable(m))
    {
        (
            m,
            format!(
                "{} task, {} specialist",
                difficulty.as_str(),
                specialty.unwrap_or("")
            ),
        )
    } else {
        let saver = if config.saver_mode {
            " (saver: one tier cheaper)"
        } else if tight {
            " (budget nearly spent: one tier cheaper)"
        } else {
            ""
        };
        (
            tier_model(router, tier)?,
            format!(
                "{} task -> {} tier{saver}",
                difficulty.as_str(),
                tier.as_str()
            ),
        )
    };
    let model = ModelRef::parse(raw).ok()?;
    Some(RouteDecision {
        model,
        difficulty,
        specialty,
        reason: why,
    })
}

/// Selects the provider for a turn and an optional note explaining the choice.
pub fn select_provider(
    config: &AppConfig,
    prompt: &str,
) -> Result<(Arc<dyn LlmProvider>, Option<String>), AiError> {
    let router = &config.router;
    let wrap = router.enabled
        || !router.fallbacks.is_empty()
        || router.max_session_usd.is_some()
        || router.max_daily_usd.is_some();
    if wrap {
        return adaptive_chain(config, route(config, prompt));
    }
    if let Some(decision) = route(config, prompt) {
        match build_provider_for(config, &decision.model) {
            Ok(provider) => {
                let note = format!("Router: {} ({})", decision.model.label(), decision.reason);
                return Ok((provider, Some(note)));
            }
            Err(err) => {
                let provider = build_provider(config)?;
                let note = format!(
                    "Router picked {} but it is unavailable ({err}); using the active model.",
                    decision.model.label()
                );
                return Ok((provider, Some(note)));
            }
        }
    }
    Ok((build_provider(config)?, None))
}

/// Providers for Swarm workers (round-robin over `[swarm].workers`) and the lead.
/// Routed model first, then stronger tiers (escalation), configured fallbacks
/// and the active model, wrapped in an [`AdaptiveProvider`](crate::ai::adaptive::AdaptiveProvider).
fn adaptive_chain(
    config: &AppConfig,
    decision: Option<RouteDecision>,
) -> Result<(Arc<dyn LlmProvider>, Option<String>), AiError> {
    use crate::ai::adaptive::{AdaptiveProvider, Candidate, StatsStore};
    let router = &config.router;
    let active = ModelRef {
        provider: config.active_provider,
        model: config.active_settings().model.clone(),
    };
    let mut note = decision
        .as_ref()
        .map(|d| format!("Router: {} ({})", d.model.label(), d.reason));
    let mut refs = vec![decision.map(|d| d.model).unwrap_or_else(|| active.clone())];
    if router.enabled {
        refs.extend([&router.standard, &router.strong].into_iter().flatten().filter_map(|r| ModelRef::parse(r).ok()));
    }
    refs.extend(router.fallbacks.iter().filter_map(|r| ModelRef::parse(r).ok()));
    refs.push(active);
    let mut chain: Vec<Candidate> = Vec::new();
    for r in refs {
        let label = r.label();
        if chain.iter().any(|c| c.label == label) {
            continue;
        }
        match build_provider_for(config, &r) {
            Ok(provider) => chain.push(Candidate { price: router.prices.get(&label).copied(), label, provider }),
            Err(err) if chain.is_empty() => {
                note = Some(format!("Router picked {label} but it is unavailable ({err})."));
            }
            Err(_) => {}
        }
    }
    if chain.is_empty() {
        return Ok((build_provider(config)?, note));
    }
    if chain.len() > 1 {
        let rest: Vec<&str> = chain[1..].iter().map(|c| c.label.as_str()).collect();
        let base = note.unwrap_or_else(|| format!("Model: {}", chain[0].label));
        note = Some(format!("{base}; fallback: {}", rest.join(" -> ")));
    }
    let provider = AdaptiveProvider::new(chain, StatsStore::shared(), router.budget());
    Ok((Arc::new(provider), note))
}

pub struct SwarmProviders {
    pub workers: Vec<Arc<dyn LlmProvider>>,
    pub lead: Option<Arc<dyn LlmProvider>>,
    pub errors: Vec<String>,
}

pub fn swarm_providers(config: &AppConfig) -> SwarmProviders {
    let mut out = SwarmProviders {
        workers: Vec::new(),
        lead: None,
        errors: Vec::new(),
    };
    let build = |raw: &str, errors: &mut Vec<String>| {
        ModelRef::parse(raw)
            .map_err(|e| format!("{raw}: {e}"))
            .and_then(|r| build_provider_for(config, &r).map_err(|e| format!("{raw}: {e}")))
            .map_err(|e| errors.push(e))
            .ok()
    };
    let mut worker_refs: Vec<String> = config.swarm_worker_models.clone();
    if worker_refs.is_empty() && config.saver_mode {
        worker_refs.extend(config.router.cheap.clone());
    }
    for raw in &worker_refs {
        if let Some(p) = build(raw, &mut out.errors) {
            out.workers.push(p);
        }
    }
    if let Some(raw) = &config.swarm_lead_model {
        out.lead = build(raw, &mut out.errors);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn config() -> AppConfig {
        let mut cfg = AppConfig::default_for_workspace(PathBuf::from("."));
        cfg.router = RouterConfig {
            enabled: true,
            cheap: Some("ollama/test-small".into()),
            standard: Some("openai/test-medium".into()),
            strong: Some("anthropic/test-large".into()),
            specialties: BTreeMap::from([(
                "security".to_string(),
                "openrouter/test/sec-model".to_string(),
            )]),
            ..Default::default()
        };
        cfg
    }

    #[test]
    fn parses_model_refs_with_nested_slashes() {
        let r = ModelRef::parse("openrouter/deepseek/deepseek-chat").unwrap();
        assert_eq!(r.provider, ProviderKind::OpenRouter);
        assert_eq!(r.model, "deepseek/deepseek-chat");
        assert_eq!(
            ModelRef::parse("ollama").unwrap().model,
            ProviderKind::Ollama.default_model()
        );
        assert!(ModelRef::parse("nope/x").is_err());
    }

    #[test]
    fn classifies_difficulty_and_specialty() {
        assert_eq!(classify("fix typo in README").0, Difficulty::Simple);
        assert_eq!(
            classify("refactor the architecture of the auth module and migrate the session store")
                .0,
            Difficulty::Complex
        );
        assert_eq!(
            classify("audit this endpoint for sql injection and xss").1,
            Some("security")
        );
    }

    #[test]
    fn routes_by_tier_specialty_and_saver_mode() {
        let mut cfg = config();
        assert_eq!(
            route(&cfg, "explain main.rs").unwrap().model.model,
            "test-small"
        );
        let complex =
            "refactor and redesign the architecture of the scheduler to fix the race condition";
        assert_eq!(route(&cfg, complex).unwrap().model.model, "test-large");
        let sec = "investigate this login handler for injection vulnerabilities and auth bypass";
        assert_eq!(route(&cfg, sec).unwrap().model.model, "test/sec-model");

        cfg.saver_mode = true;
        assert_eq!(route(&cfg, complex).unwrap().model.model, "test-medium");

        cfg.router.enabled = false;
        assert_eq!(route(&cfg, complex).unwrap().model.model, "test-small");
        cfg.saver_mode = false;
        assert!(route(&cfg, complex).is_none());
    }

    #[test]
    fn swarm_providers_mix_labs_and_report_bad_refs() {
        let mut cfg = config();
        cfg.swarm_worker_models = vec!["ollama/a".into(), "ollama/b".into(), "bogus/x".into()];
        cfg.swarm_lead_model = Some("ollama/lead".into());
        let p = swarm_providers(&cfg);
        assert_eq!(p.workers.len(), 2);
        assert_eq!(p.workers[1].model(), "b");
        assert_eq!(p.lead.unwrap().model(), "lead");
        assert_eq!(p.errors.len(), 1);
    }

    #[test]
    fn adaptive_router_parses_budgets_and_builds_a_fallback_chain() {
        let toml_src = r#"
enabled = true
cheap = "ollama/test-small"
strong = "ollama/test-large"
fallbacks = ["ollama/test-backup"]
max_daily_usd = 5.0
[prices]
"ollama/test-large" = { input = 3.0, output = 15.0 }
"#;
        let router: RouterConfig = toml::from_str(toml_src).unwrap();
        assert!(router.is_adaptive());
        assert_eq!(router.budget().max_daily_usd, Some(5.0));
        assert_eq!(router.prices["ollama/test-large"].output, 15.0);
        let mut cfg = config();
        cfg.router = router;
        let (provider, note) = select_provider(&cfg, "rename this variable").unwrap();
        let note = note.unwrap();
        assert!(note.contains("fallback:") && note.contains("ollama/test-backup"), "{note}");
        assert_eq!(provider.model(), "test-small");
    }
}
