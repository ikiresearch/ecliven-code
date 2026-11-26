//! Optional embeddings for semantic search. Vectors are keyed by chunk hash, so
//! only chunks whose content changed are embedded again.

use std::fmt;

use rusqlite::params;
use serde_json::{json, Value};

use super::{err, KResult, KnowledgeStore};
use crate::config::{AppConfig, ProviderKind};

const BATCH: usize = 64;
/// Upper bound of new chunks embedded per query, to cap cost on large repositories.
pub const MAX_NEW_PER_QUERY: usize = 512;

#[derive(Clone)]
pub struct EmbeddingClient {
    ollama: bool,
    base_url: String,
    pub model: String,
    api_key: Option<String>,
}

impl fmt::Debug for EmbeddingClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EmbeddingClient")
            .field("model", &self.model)
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl EmbeddingClient {
    /// Builds a client from a `provider/model` ref (ollama, openai or openrouter).
    pub fn from_config(config: &AppConfig, raw: &str) -> Result<Self, String> {
        let r = crate::ai::router::ModelRef::parse(raw)?;
        let settings = config
            .providers
            .get(&r.provider)
            .ok_or_else(|| format!("provider {} is not configured", r.provider.as_str()))?;
        let ollama = match r.provider {
            ProviderKind::Ollama => true,
            ProviderKind::OpenAi | ProviderKind::OpenRouter => false,
            other => return Err(format!("{} does not provide an embeddings API", other.as_str())),
        };
        if !ollama && settings.api_key.is_none() {
            return Err(format!("{} needs an API key for embeddings", r.provider.as_str()));
        }
        Ok(Self {
            ollama,
            base_url: settings.base_url.trim_end_matches('/').to_string(),
            model: r.model,
            api_key: settings.api_key.clone(),
        })
    }

    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let client = reqwest::Client::new();
        let (url, body) = if self.ollama {
            (format!("{}/api/embed", self.base_url), json!({ "model": self.model, "input": texts }))
        } else {
            (format!("{}/embeddings", self.base_url), json!({ "model": self.model, "input": texts }))
        };
        let mut req = client.post(url).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = req.send().await.map_err(err)?;
        let status = resp.status();
        let value: Value = resp.json().await.map_err(err)?;
        if !status.is_success() {
            return Err(format!("embeddings request failed ({status}): {value}"));
        }
        let vectors: Vec<Vec<f32>> = if self.ollama {
            value["embeddings"].as_array().cloned().unwrap_or_default()
        } else {
            value["data"]
                .as_array()
                .map(|d| d.iter().map(|e| e["embedding"].clone()).collect())
                .unwrap_or_default()
        }
        .into_iter()
        .map(|v| {
            v.as_array()
                .map(|a| a.iter().filter_map(Value::as_f64).map(|x| x as f32).collect())
                .unwrap_or_default()
        })
        .collect();
        if vectors.len() != texts.len() {
            return Err("embeddings response size mismatch".into());
        }
        Ok(vectors)
    }
}

pub(crate) fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub(crate) fn from_blob(b: &[u8]) -> Vec<f32> {
    b.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect()
}

pub(crate) fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for (x, y) in a.iter().zip(b) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

impl KnowledgeStore {
    /// Chunks (hash, text) that have no vector for `model` yet.
    pub fn chunks_missing_embeddings(&self, model: &str, limit: usize) -> KResult<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT c.hash, MIN(f.text) FROM chunks c JOIN chunks_fts f ON f.rowid = c.id
                 WHERE NOT EXISTS (SELECT 1 FROM embeddings e WHERE e.hash = c.hash AND e.model = ?1)
                 GROUP BY c.hash LIMIT ?2",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map(params![model, limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    pub fn store_embeddings(&self, model: &str, items: &[(String, Vec<f32>)]) -> KResult<()> {
        for (hash, v) in items {
            self.conn
                .execute(
                    "INSERT OR REPLACE INTO embeddings (hash, model, vector) VALUES (?1, ?2, ?3)",
                    params![hash, model, to_blob(v)],
                )
                .map_err(err)?;
        }
        Ok(())
    }

    /// Nearest chunks to `query_vec` by cosine similarity.
    pub fn search_vectors(&self, model: &str, query_vec: &[f32], limit: usize) -> KResult<Vec<super::CodeHit>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT c.path, c.start_line, c.end_line, e.vector FROM embeddings e
                 JOIN chunks c ON c.hash = e.hash WHERE e.model = ?1",
            )
            .map_err(err)?;
        let rows = stmt
            .query_map(params![model], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Vec<u8>>(3)?,
                ))
            })
            .map_err(err)?;
        let mut hits = Vec::new();
        for row in rows {
            let (path, start_line, end_line, blob) = row.map_err(err)?;
            hits.push(super::CodeHit {
                path,
                start_line,
                end_line,
                snippet: String::new(),
                score: cosine(query_vec, &from_blob(&blob)) as f64,
            });
        }
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(limit);
        Ok(hits)
    }
}

/// Embeds new chunks (bounded) and returns semantic hits for `query`.
pub async fn semantic_search(
    client: &EmbeddingClient,
    workspace: &std::path::Path,
    query: &str,
    limit: usize,
) -> Result<Vec<super::CodeHit>, String> {
    let ws = workspace.to_path_buf();
    let model = client.model.clone();
    let missing = {
        let (ws, model) = (ws.clone(), model.clone());
        tokio::task::spawn_blocking(move || {
            KnowledgeStore::open(&ws)?.chunks_missing_embeddings(&model, MAX_NEW_PER_QUERY)
        })
        .await
        .map_err(err)??
    };
    for batch in missing.chunks(BATCH) {
        let texts: Vec<String> = batch
            .iter()
            .map(|(_, t)| crate::tools::truncate_str(t, 6000).to_string())
            .collect();
        let vectors = client.embed(&texts).await?;
        let items: Vec<(String, Vec<f32>)> = batch.iter().map(|(h, _)| h.clone()).zip(vectors).collect();
        let (ws, model) = (ws.clone(), model.clone());
        tokio::task::spawn_blocking(move || KnowledgeStore::open(&ws)?.store_embeddings(&model, &items))
            .await
            .map_err(err)??;
    }
    let qv = client.embed(&[query.to_string()]).await?.pop().unwrap_or_default();
    tokio::task::spawn_blocking(move || KnowledgeStore::open(&ws)?.search_vectors(&model, &qv, limit))
        .await
        .map_err(err)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_round_trip_and_rank() {
        let v = vec![0.5f32, -1.0, 2.0];
        assert_eq!(from_blob(&to_blob(&v)), v);
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
    }

    #[test]
    fn only_new_chunks_need_embeddings() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(ws.join("a.rs"), "fn a() {}\n").unwrap();
        let mut s = KnowledgeStore::open_at(&tmp.path().join("k.db"), &ws).unwrap();
        s.refresh().unwrap();
        let missing = s.chunks_missing_embeddings("m", 10).unwrap();
        assert_eq!(missing.len(), 1);
        s.store_embeddings("m", &[(missing[0].0.clone(), vec![1.0, 0.0])]).unwrap();
        assert!(s.chunks_missing_embeddings("m", 10).unwrap().is_empty());
        std::fs::write(ws.join("b.rs"), "fn b() {}\n").unwrap();
        s.refresh().unwrap();
        assert_eq!(s.chunks_missing_embeddings("m", 10).unwrap().len(), 1);
        assert_eq!(s.search_vectors("m", &[1.0, 0.0], 5).unwrap()[0].path, "a.rs");
    }
}
