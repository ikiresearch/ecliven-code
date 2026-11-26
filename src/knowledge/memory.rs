//! Layered memories with provenance, and task history / active plan.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{err, fts_query, hash_bytes, now, KResult, KnowledgeStore};

pub const MEMORY_KINDS: &[&str] = &["fact", "decision", "convention", "gotcha", "preference"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Memory {
    pub id: i64,
    pub scope: String,
    pub kind: String,
    pub text: String,
    pub stale: bool,
    pub stale_reason: Option<String>,
    pub sources: Vec<String>,
}

impl Memory {
    /// One line with provenance, e.g. `[m3 project/convention] text (src: a.rs) (possibly stale: …)`.
    pub fn render(&self) -> String {
        let prefix = if self.scope == "user" { "u" } else { "m" };
        let mut line = format!("[{prefix}{} {}/{}] {}", self.id, self.scope, self.kind, self.text);
        if !self.sources.is_empty() {
            line.push_str(&format!(" (src: {})", self.sources.join(", ")));
        }
        if self.stale {
            line.push_str(&format!(
                " (POSSIBLY STALE: {})",
                self.stale_reason.as_deref().unwrap_or("source changed")
            ));
        }
        line
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanStep {
    pub step: String,
    #[serde(default = "pending")]
    pub status: String,
}

fn pending() -> String {
    "pending".into()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskRecord {
    pub id: i64,
    pub goal: String,
    pub status: String,
    pub plan: Vec<PlanStep>,
    pub files: Vec<String>,
    pub summary: String,
    pub active: bool,
}

impl TaskRecord {
    pub fn render_plan(&self) -> String {
        self.plan
            .iter()
            .map(|s| {
                let mark = match s.status.as_str() {
                    "done" | "completed" => "x",
                    "in_progress" => ">",
                    _ => " ",
                };
                format!("[{mark}] {}", s.step)
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}

const MEMORY_COLS: &str = "m.id, m.scope, m.kind, m.text, m.stale, m.stale_reason";

impl KnowledgeStore {
    /// Saves a memory; `sources` are workspace-relative paths whose current hash is recorded.
    pub fn add_memory(&self, scope: &str, kind: &str, text: &str, sources: &[String]) -> KResult<i64> {
        let text = text.trim();
        if text.is_empty() {
            return Err("memory text is empty".into());
        }
        let kind = if MEMORY_KINDS.contains(&kind) { kind } else { "fact" };
        if let Some(existing) = self
            .conn
            .query_row("SELECT id FROM memories WHERE text = ?1", params![text], |r| r.get::<_, i64>(0))
            .optional()
            .map_err(err)?
        {
            return Ok(existing);
        }
        let ts = now();
        self.conn
            .execute(
                "INSERT INTO memories (scope, kind, text, created_at, used_at) VALUES (?1, ?2, ?3, ?4, ?4)",
                params![scope, kind, text, ts],
            )
            .map_err(err)?;
        let id = self.conn.last_insert_rowid();
        self.conn
            .execute("INSERT INTO memories_fts (rowid, text) VALUES (?1, ?2)", params![id, text])
            .map_err(err)?;
        for src in sources {
            let rel = src.trim().trim_start_matches("./").replace('\\', "/");
            if rel.is_empty() {
                continue;
            }
            let hash = match self.file_hash(&rel)? {
                Some(h) => h,
                None => std::fs::read(self.workspace.join(&rel))
                    .map(|b| hash_bytes(&b))
                    .unwrap_or_default(),
            };
            self.conn
                .execute(
                    "INSERT INTO memory_sources (memory_id, path, hash) VALUES (?1, ?2, ?3)",
                    params![id, rel, hash],
                )
                .map_err(err)?;
        }
        Ok(id)
    }

    fn sources_of(&self, id: i64) -> KResult<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM memory_sources WHERE memory_id = ?1")
            .map_err(err)?;
        let rows = stmt.query_map(params![id], |r| r.get(0)).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    fn collect_memories(&self, sql: &str, p: &[&dyn rusqlite::ToSql]) -> KResult<Vec<Memory>> {
        let mut stmt = self.conn.prepare(sql).map_err(err)?;
        let rows = stmt
            .query_map(p, |r| {
                Ok(Memory {
                    id: r.get(0)?,
                    scope: r.get(1)?,
                    kind: r.get(2)?,
                    text: r.get(3)?,
                    stale: r.get::<_, i64>(4)? != 0,
                    stale_reason: r.get(5)?,
                    sources: Vec::new(),
                })
            })
            .map_err(err)?;
        let mut out: Vec<Memory> = rows.collect::<Result<_, _>>().map_err(err)?;
        for m in &mut out {
            m.sources = self.sources_of(m.id)?;
        }
        Ok(out)
    }

    /// Relevant memories for `query` (fresh before stale); recent ones if the query is empty.
    pub fn search_memories(&self, query: &str, limit: usize) -> KResult<Vec<Memory>> {
        let found = match fts_query(query, 12) {
            Some(q) => self.collect_memories(
                &format!(
                    "SELECT {MEMORY_COLS} FROM memories_fts JOIN memories m ON m.id = memories_fts.rowid
                     WHERE memories_fts MATCH ?1 ORDER BY m.stale, bm25(memories_fts) LIMIT ?2"
                ),
                &[&q, &(limit as i64)],
            )?,
            None => self.list_memories(limit, false)?,
        };
        let ts = now();
        for m in &found {
            let _ = self
                .conn
                .execute("UPDATE memories SET used_at = ?2 WHERE id = ?1", params![m.id, ts]);
        }
        Ok(found)
    }

    pub fn list_memories(&self, limit: usize, only_stale: bool) -> KResult<Vec<Memory>> {
        let filter = if only_stale { "WHERE m.stale = 1" } else { "" };
        self.collect_memories(
            &format!("SELECT {MEMORY_COLS} FROM memories m {filter} ORDER BY m.used_at DESC, m.id DESC LIMIT ?1"),
            &[&(limit as i64)],
        )
    }

    pub fn forget_memory(&self, id: i64) -> KResult<bool> {
        self.conn
            .execute("DELETE FROM memory_sources WHERE memory_id = ?1", params![id])
            .map_err(err)?;
        self.conn
            .execute("DELETE FROM memories_fts WHERE rowid = ?1", params![id])
            .map_err(err)?;
        let n = self
            .conn
            .execute("DELETE FROM memories WHERE id = ?1", params![id])
            .map_err(err)?;
        Ok(n > 0)
    }

    /// Marks a memory as verified against the current sources.
    pub fn confirm_memory(&self, id: i64) -> KResult<bool> {
        for src in self.sources_of(id)? {
            if let Some(hash) = self.file_hash(&src)? {
                self.conn
                    .execute(
                        "UPDATE memory_sources SET hash = ?3 WHERE memory_id = ?1 AND path = ?2",
                        params![id, src, hash],
                    )
                    .map_err(err)?;
            }
        }
        let n = self
            .conn
            .execute("UPDATE memories SET stale = 0, stale_reason = NULL WHERE id = ?1", params![id])
            .map_err(err)?;
        Ok(n > 0)
    }

    fn task_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRecord> {
        let plan: String = r.get(3)?;
        let files: String = r.get(4)?;
        Ok(TaskRecord {
            id: r.get(0)?,
            goal: r.get(1)?,
            status: r.get(2)?,
            plan: serde_json::from_str(&plan).unwrap_or_default(),
            files: serde_json::from_str(&files).unwrap_or_default(),
            summary: r.get(5)?,
            active: r.get::<_, i64>(6)? != 0,
        })
    }

    const TASK_COLS: &'static str = "id, goal, status, plan, files, summary, active";

    pub fn active_task(&self) -> KResult<Option<TaskRecord>> {
        self.conn
            .query_row(
                &format!("SELECT {} FROM tasks WHERE active = 1 ORDER BY id DESC LIMIT 1", Self::TASK_COLS),
                [],
                Self::task_from_row,
            )
            .optional()
            .map_err(err)
    }

    pub fn recent_tasks(&self, limit: usize) -> KResult<Vec<TaskRecord>> {
        let mut stmt = self
            .conn
            .prepare(&format!("SELECT {} FROM tasks ORDER BY updated_at DESC, id DESC LIMIT ?1", Self::TASK_COLS))
            .map_err(err)?;
        let rows = stmt.query_map(params![limit as i64], Self::task_from_row).map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }

    /// Starts a new active task, closing any previous one.
    pub fn start_task(&self, goal: &str) -> KResult<i64> {
        let ts = now();
        self.conn
            .execute("UPDATE tasks SET active = 0, status = CASE WHEN status = 'active' THEN 'paused' ELSE status END WHERE active = 1", [])
            .map_err(err)?;
        self.conn
            .execute(
                "INSERT INTO tasks (goal, status, active, created_at, updated_at) VALUES (?1, 'active', 1, ?2, ?2)",
                params![goal.trim(), ts],
            )
            .map_err(err)?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Updates the active task's plan (creating the task when `goal` is given and none is active).
    pub fn update_plan(&self, goal: Option<&str>, steps: &[PlanStep], status: Option<&str>) -> KResult<TaskRecord> {
        let task = match (self.active_task()?, goal) {
            (Some(t), Some(g)) if t.goal != g.trim() && !g.trim().is_empty() => {
                self.start_task(g)?;
                self.active_task()?
            }
            (Some(t), _) => Some(t),
            (None, Some(g)) if !g.trim().is_empty() => {
                self.start_task(g)?;
                self.active_task()?
            }
            (None, _) => None,
        }
        .ok_or("no active task: pass a goal to start one")?;
        let plan = if steps.is_empty() { task.plan.clone() } else { steps.to_vec() };
        let finished = matches!(status, Some("done" | "completed" | "abandoned"));
        let status = status.map(|s| if s == "completed" { "done" } else { s }).unwrap_or(&task.status);
        self.conn
            .execute(
                "UPDATE tasks SET plan = ?2, status = ?3, active = ?4, updated_at = ?5 WHERE id = ?1",
                params![
                    task.id,
                    serde_json::to_string(&plan).map_err(err)?,
                    status,
                    if finished { 0 } else { 1 },
                    now()
                ],
            )
            .map_err(err)?;
        Ok(TaskRecord {
            plan,
            status: status.to_string(),
            active: !finished,
            ..task
        })
    }

    /// Records a finished agent turn: attaches it to the active task or logs a one-off task.
    pub fn record_turn(&self, request: &str, files: &[String], summary: &str, ok: bool) -> KResult<i64> {
        let summary = crate::tools::truncate_str(summary.trim(), 400).to_string();
        let ts = now();
        if let Some(task) = self.active_task()? {
            let mut all = task.files.clone();
            for f in files {
                if !all.contains(f) {
                    all.push(f.clone());
                }
            }
            self.conn
                .execute(
                    "UPDATE tasks SET files = ?2, summary = ?3, updated_at = ?4 WHERE id = ?1",
                    params![task.id, serde_json::to_string(&all).map_err(err)?, summary, ts],
                )
                .map_err(err)?;
            return Ok(task.id);
        }
        let goal = crate::tools::truncate_str(request.lines().next().unwrap_or("").trim(), 120).to_string();
        self.conn
            .execute(
                "INSERT INTO tasks (goal, status, files, summary, active, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 0, ?5, ?5)",
                params![
                    goal,
                    if ok { "done" } else { "error" },
                    serde_json::to_string(files).map_err(err)?,
                    summary,
                    ts
                ],
            )
            .map_err(err)?;
        Ok(self.conn.last_insert_rowid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn store(tmp: &TempDir) -> KnowledgeStore {
        let ws = tmp.path().join("ws");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::write(ws.join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
        let mut s = KnowledgeStore::open_at(&tmp.path().join("k.db"), &ws).unwrap();
        s.refresh().unwrap();
        s
    }

    #[test]
    fn memories_are_searchable_and_go_stale_when_sources_change() {
        let tmp = TempDir::new().unwrap();
        let mut s = store(&tmp);
        let id = s
            .add_memory("project", "convention", "alpha must stay allocation free", &["src/lib.rs".into()])
            .unwrap();
        assert_eq!(s.add_memory("project", "fact", "alpha must stay allocation free", &[]).unwrap(), id);
        let found = s.search_memories("is alpha allocation free?", 5).unwrap();
        assert_eq!(found.len(), 1);
        assert!(!found[0].stale);
        assert_eq!(found[0].sources, vec!["src/lib.rs"]);

        std::fs::write(s.workspace().join("src/lib.rs"), "pub fn alpha() { let _v = vec![1]; }\n").unwrap();
        let stats = s.refresh().unwrap();
        assert_eq!((stats.updated, stats.stale_memories), (1, 1));
        let m = &s.search_memories("alpha", 5).unwrap()[0];
        assert!(m.stale && m.render().contains("POSSIBLY STALE"));

        assert!(s.confirm_memory(id).unwrap());
        assert!(!s.search_memories("alpha", 5).unwrap()[0].stale);
        assert!(s.forget_memory(id).unwrap());
        assert!(s.search_memories("alpha", 5).unwrap().is_empty());
    }

    #[test]
    fn renames_keep_memories_and_deletes_mark_them_stale() {
        let tmp = TempDir::new().unwrap();
        let mut s = store(&tmp);
        std::fs::write(s.workspace().join("src/other.rs"), "pub fn beta() {}\n").unwrap();
        s.refresh().unwrap();
        s.add_memory("project", "fact", "alpha lives in lib", &["src/lib.rs".into()]).unwrap();
        s.add_memory("project", "fact", "beta lives in other", &["src/other.rs".into()]).unwrap();

        let ws = s.workspace().to_path_buf();
        std::fs::rename(ws.join("src/lib.rs"), ws.join("src/core.rs")).unwrap();
        std::fs::remove_file(ws.join("src/other.rs")).unwrap();
        let stats = s.refresh().unwrap();
        assert_eq!((stats.renamed, stats.removed, stats.stale_memories), (1, 1, 1));
        assert_eq!(s.search_memories("alpha", 5).unwrap()[0].sources, vec!["src/core.rs"]);
        assert!(s.search_memories("beta", 5).unwrap()[0].stale);
        assert!(s.find_symbols("beta", 5).unwrap().is_empty());
        assert_eq!(s.find_symbols("alpha", 5).unwrap()[0].path, "src/core.rs");
    }

    #[test]
    fn tasks_track_plan_and_turns() {
        let tmp = TempDir::new().unwrap();
        let s = store(&tmp);
        s.record_turn("rename foo\nmore", &["a.rs".into()], "renamed", true).unwrap();
        let steps = vec![
            PlanStep { step: "inspect".into(), status: "done".into() },
            PlanStep { step: "edit".into(), status: "in_progress".into() },
        ];
        let t = s.update_plan(Some("add cache"), &steps, None).unwrap();
        assert!(t.active);
        assert_eq!(t.render_plan(), "[x] inspect; [>] edit");
        s.record_turn("next", &["b.rs".into()], "edited", true).unwrap();
        s.record_turn("next", &["b.rs".into(), "c.rs".into()], "tested", true).unwrap();
        assert_eq!(s.active_task().unwrap().unwrap().files, vec!["b.rs", "c.rs"]);
        s.update_plan(None, &[], Some("completed")).unwrap();
        assert!(s.active_task().unwrap().is_none());
        let recent = s.recent_tasks(5).unwrap();
        assert_eq!(recent.len(), 2);
        assert!(recent.iter().any(|t| t.goal == "rename foo" && t.status == "done"));
        assert!(s.update_plan(None, &steps, None).is_err());
    }
}
