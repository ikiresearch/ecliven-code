//! Adaptive routing: per-model statistics learned from real calls (recent
//! success rate, latency, tokens, cost), spending budgets, and a provider that
//! falls back or escalates along a chain of models on rate limits, outages or
//! context overflows.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use super::{AiError, CompletionRequest, CompletionResponse, LlmProvider, TokenUsage};

const RECENT: usize = 20;
const MIN_SAMPLES: usize = 5;
pub const MIN_SUCCESS: f64 = 0.6;
const RATE_LIMIT_COOLDOWN: u64 = 60;
const OUTAGE_COOLDOWN: u64 = 30;

/// USD per million tokens.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq)]
pub struct Price {
    pub input: f64,
    pub output: f64,
}

impl Price {
    /// Cached prompt tokens are billed at a tenth of the input price.
    pub fn cost(&self, u: &TokenUsage) -> f64 {
        let cached = u.cache_read_tokens.min(u.input_tokens) as f64;
        let fresh = u.input_tokens as f64 - cached;
        (fresh * self.input + cached * self.input * 0.1 + u.output_tokens as f64 * self.output) / 1e6
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelStats {
    pub calls: u64,
    pub failures: u64,
    pub rate_limited: u64,
    pub latency_ms_avg: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: f64,
    #[serde(default)]
    pub cooldown_until: u64,
    /// Outcome of the last calls, newest last.
    #[serde(default)]
    pub recent: Vec<bool>,
}

impl ModelStats {
    pub fn success_rate(&self) -> Option<f64> {
        (self.recent.len() >= MIN_SAMPLES)
            .then(|| self.recent.iter().filter(|ok| **ok).count() as f64 / self.recent.len() as f64)
    }

    pub fn cooling(&self, now: u64) -> bool {
        self.cooldown_until > now
    }

    fn push(&mut self, ok: bool) {
        self.recent.push(ok);
        if self.recent.len() > RECENT {
            self.recent.remove(0);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    RateLimited,
    Unavailable,
    ContextTooLong,
    Fatal,
}

pub fn classify_error(e: &AiError) -> Failure {
    match e {
        AiError::ApiStatus { status: 429, .. } => Failure::RateLimited,
        AiError::ApiStatus { status, .. } if *status >= 500 || *status == 408 || *status == 529 => Failure::Unavailable,
        AiError::ApiStatus { status: 400 | 413, body } => {
            let b = body.to_ascii_lowercase();
            if ["context", "too long", "too many tokens", "maximum"].iter().any(|k| b.contains(k)) {
                Failure::ContextTooLong
            } else {
                Failure::Fatal
            }
        }
        AiError::Http(_) => Failure::Unavailable,
        AiError::CliFailed { output, .. } => {
            let o = output.to_ascii_lowercase();
            if ["rate limit", "429", "quota", "overloaded"].iter().any(|k| o.contains(k)) {
                Failure::RateLimited
            } else {
                Failure::Fatal
            }
        }
        _ => Failure::Fatal,
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StatsFile {
    #[serde(default)]
    models: BTreeMap<String, ModelStats>,
    /// Day number since the epoch -> USD spent.
    #[serde(default)]
    daily_cost: BTreeMap<u64, f64>,
}

pub struct StatsStore {
    path: Option<PathBuf>,
    data: Mutex<StatsFile>,
    session_cost: Mutex<f64>,
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn today() -> u64 {
    now_secs() / 86_400
}

impl StatsStore {
    pub fn in_memory() -> Arc<Self> {
        Arc::new(Self { path: None, data: Mutex::new(StatsFile::default()), session_cost: Mutex::new(0.0) })
    }

    pub fn open_at(path: PathBuf) -> Arc<Self> {
        let data = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Arc::new(Self { path: Some(path), data: Mutex::new(data), session_cost: Mutex::new(0.0) })
    }

    /// Process-wide store persisted in the user data directory.
    pub fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<StatsStore>> = OnceLock::new();
        SHARED
            .get_or_init(|| {
                if cfg!(test) {
                    return Self::in_memory();
                }
                match dirs::data_dir() {
                    Some(d) => Self::open_at(d.join("ecliven-code").join("router-stats.json")),
                    None => Self::in_memory(),
                }
            })
            .clone()
    }

    fn save(&self, data: &StatsFile) {
        let Some(path) = &self.path else { return };
        let Ok(text) = serde_json::to_string_pretty(data) else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    fn update(&self, f: impl FnOnce(&mut StatsFile)) {
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut data);
        let snapshot = data.clone();
        drop(data);
        self.save(&snapshot);
    }

    pub fn get(&self, label: &str) -> ModelStats {
        let data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        data.models.get(label).cloned().unwrap_or_default()
    }

    pub fn snapshot(&self) -> BTreeMap<String, ModelStats> {
        self.data.lock().unwrap_or_else(|e| e.into_inner()).models.clone()
    }

    pub fn reset(&self) {
        self.update(|d| d.models.clear());
    }

    pub fn record_success(&self, label: &str, latency_ms: f64, usage: &TokenUsage, cost: f64) {
        *self.session_cost.lock().unwrap_or_else(|e| e.into_inner()) += cost;
        self.update(|d| {
            let s = d.models.entry(label.to_string()).or_default();
            s.calls += 1;
            s.latency_ms_avg = if s.calls == 1 { latency_ms } else { s.latency_ms_avg * 0.7 + latency_ms * 0.3 };
            s.input_tokens += u64::from(usage.input_tokens);
            s.output_tokens += u64::from(usage.output_tokens);
            s.cost_usd += cost;
            s.cooldown_until = 0;
            s.push(true);
            if cost > 0.0 {
                *d.daily_cost.entry(today()).or_default() += cost;
                let keep = today().saturating_sub(30);
                d.daily_cost.retain(|day, _| *day >= keep);
            }
        });
    }

    pub fn record_failure(&self, label: &str, failure: Failure) {
        self.update(|d| {
            let s = d.models.entry(label.to_string()).or_default();
            s.calls += 1;
            s.failures += 1;
            match failure {
                Failure::RateLimited => {
                    s.rate_limited += 1;
                    s.cooldown_until = now_secs() + RATE_LIMIT_COOLDOWN;
                }
                Failure::Unavailable => s.cooldown_until = now_secs() + OUTAGE_COOLDOWN,
                Failure::ContextTooLong | Failure::Fatal => {}
            }
            if failure != Failure::ContextTooLong {
                s.push(false);
            }
        });
    }

    pub fn spent_today(&self) -> f64 {
        let data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        data.daily_cost.get(&today()).copied().unwrap_or(0.0)
    }

    pub fn spent_session(&self) -> f64 {
        *self.session_cost.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Usable now: not cooling down, recent success rate above the floor and,
    /// when a latency cap is set, average latency under it.
    pub fn healthy(&self, label: &str, max_latency_ms: Option<u64>) -> bool {
        let s = self.get(label);
        !s.cooling(now_secs())
            && s.success_rate().is_none_or(|r| r >= MIN_SUCCESS)
            && max_latency_ms.is_none_or(|cap| s.calls == 0 || s.latency_ms_avg <= cap as f64)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Budget {
    pub max_session_usd: Option<f64>,
    pub max_daily_usd: Option<f64>,
}

impl Budget {
    pub fn exhausted(&self, stats: &StatsStore) -> Option<String> {
        if let Some(max) = self.max_session_usd {
            if stats.spent_session() >= max {
                return Some(format!("session budget of ${max:.2} reached"));
            }
        }
        if let Some(max) = self.max_daily_usd {
            if stats.spent_today() >= max {
                return Some(format!("daily budget of ${max:.2} reached"));
            }
        }
        None
    }

    /// True once 80% of a budget is spent: routing drops one tier.
    pub fn nearly_exhausted(&self, stats: &StatsStore) -> bool {
        self.max_session_usd.is_some_and(|m| stats.spent_session() >= m * 0.8)
            || self.max_daily_usd.is_some_and(|m| stats.spent_today() >= m * 0.8)
    }
}

pub struct Candidate {
    pub label: String,
    pub provider: Arc<dyn LlmProvider>,
    pub price: Option<Price>,
}

/// Tries each candidate in order: skips models cooling down or (when a budget
/// is exhausted) paid models; on 429/outage falls back, on context overflow
/// escalates to the next (larger) model; other errors are returned as-is.
pub struct AdaptiveProvider {
    chain: Vec<Candidate>,
    stats: Arc<StatsStore>,
    budget: Budget,
}

impl AdaptiveProvider {
    pub fn new(chain: Vec<Candidate>, stats: Arc<StatsStore>, budget: Budget) -> Self {
        Self { chain, stats, budget }
    }

    pub fn labels(&self) -> Vec<&str> {
        self.chain.iter().map(|c| c.label.as_str()).collect()
    }
}

#[async_trait]
impl LlmProvider for AdaptiveProvider {
    fn name(&self) -> &'static str {
        self.chain.first().map(|c| c.provider.name()).unwrap_or("router")
    }

    fn model(&self) -> &str {
        self.chain.first().map(|c| c.provider.model()).unwrap_or("")
    }

    async fn complete(
        &self,
        request: &CompletionRequest,
        delta_tx: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<CompletionResponse, AiError> {
        let now = now_secs();
        let budget_hit = self.budget.exhausted(&self.stats);
        let mut last_err: Option<AiError> = None;
        let mut skipped = Vec::new();
        for (i, c) in self.chain.iter().enumerate() {
            let is_last = i + 1 == self.chain.len();
            let paid = c.price.is_some_and(|p| p.input > 0.0 || p.output > 0.0);
            if budget_hit.is_some() && paid {
                skipped.push(format!("{} (budget)", c.label));
                continue;
            }
            if self.stats.get(&c.label).cooling(now) && !(is_last && last_err.is_none() && skipped.len() == i) {
                skipped.push(format!("{} (cooling down)", c.label));
                continue;
            }
            let started = Instant::now();
            match c.provider.complete(request, delta_tx.clone()).await {
                Ok(resp) => {
                    let usage = resp.usage.clone().unwrap_or_default();
                    let cost = c.price.map(|p| p.cost(&usage)).unwrap_or(0.0);
                    self.stats.record_success(&c.label, started.elapsed().as_millis() as f64, &usage, cost);
                    return Ok(resp);
                }
                Err(e) => {
                    let failure = classify_error(&e);
                    self.stats.record_failure(&c.label, failure);
                    if failure == Failure::Fatal {
                        return Err(e);
                    }
                    last_err = Some(e);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            AiError::Parse(format!(
                "no model available: {}{}",
                skipped.join(", "),
                budget_hit.map(|b| format!("; {b}")).unwrap_or_default()
            ))
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fake {
        model: &'static str,
        fail_with: Option<u16>,
        calls: AtomicUsize,
    }

    #[async_trait]
    impl LlmProvider for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        fn model(&self) -> &str {
            self.model
        }
        async fn complete(
            &self,
            _r: &CompletionRequest,
            _d: Option<mpsc::UnboundedSender<String>>,
        ) -> Result<CompletionResponse, AiError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.fail_with {
                Some(status) => Err(AiError::ApiStatus { status, body: "context length exceeded".into() }),
                None => Ok(CompletionResponse {
                    content: self.model.into(),
                    usage: Some(TokenUsage { input_tokens: 1_000_000, output_tokens: 0, ..Default::default() }),
                    ..Default::default()
                }),
            }
        }
    }

    fn fake(model: &'static str, fail_with: Option<u16>) -> Arc<Fake> {
        Arc::new(Fake { model, fail_with, calls: AtomicUsize::new(0) })
    }

    fn cand(label: &str, p: Arc<Fake>, price: Option<Price>) -> Candidate {
        Candidate { label: label.into(), provider: p, price }
    }

    fn request() -> CompletionRequest {
        CompletionRequest { system_prompt: String::new(), messages: Vec::new(), tools: Vec::new(), max_tokens: 10, temperature: 0.0 }
    }

    #[tokio::test]
    async fn falls_back_on_429_and_cools_the_model_down() {
        let stats = StatsStore::in_memory();
        let (a, b) = (fake("a", Some(429)), fake("b", None));
        let p = AdaptiveProvider::new(vec![cand("x/a", a.clone(), None), cand("x/b", b.clone(), None)], stats.clone(), Budget::default());
        assert_eq!(p.complete(&request(), None).await.unwrap().content, "b");
        assert!(stats.get("x/a").cooling(now_secs()));
        assert_eq!(p.complete(&request(), None).await.unwrap().content, "b");
        assert_eq!(a.calls.load(Ordering::SeqCst), 1, "cooling model is skipped");
    }

    #[tokio::test]
    async fn escalates_on_context_overflow_and_stops_on_fatal_errors() {
        let stats = StatsStore::in_memory();
        let p = AdaptiveProvider::new(vec![cand("x/small", fake("small", Some(400)), None), cand("x/big", fake("big", None), None)], stats.clone(), Budget::default());
        assert_eq!(p.complete(&request(), None).await.unwrap().content, "big");
        let fatal = AdaptiveProvider::new(vec![cand("x/a", fake("a", Some(401)), None), cand("x/b", fake("b", None), None)], stats, Budget::default());
        assert!(matches!(fatal.complete(&request(), None).await, Err(AiError::ApiStatus { status: 401, .. })));
    }

    #[tokio::test]
    async fn budget_tracks_cost_and_then_only_allows_free_models() {
        let stats = StatsStore::in_memory();
        let price = Some(Price { input: 2.0, output: 0.0 });
        let budget = Budget { max_session_usd: Some(1.5), max_daily_usd: None };
        let paid = fake("paid", None);
        let p = AdaptiveProvider::new(vec![cand("x/paid", paid.clone(), price), cand("local/free", fake("free", None), None)], stats.clone(), budget);
        assert_eq!(p.complete(&request(), None).await.unwrap().content, "paid");
        assert!((stats.spent_session() - 2.0).abs() < 1e-9);
        assert!(budget.nearly_exhausted(&stats));
        assert_eq!(p.complete(&request(), None).await.unwrap().content, "free");
        assert_eq!(paid.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn learns_health_from_recent_outcomes() {
        let stats = StatsStore::in_memory();
        for _ in 0..5 {
            stats.record_failure("x/flaky", Failure::Fatal);
        }
        assert!(!stats.healthy("x/flaky", None));
        stats.record_success("x/slow", 9000.0, &TokenUsage::default(), 0.0);
        assert!(!stats.healthy("x/slow", Some(5000)) && stats.healthy("x/slow", None));
        assert!(stats.healthy("x/new", Some(1)));
    }
}
