//! `/memory`, `/index`, `/tasks` and `/goal` command handlers shared by the TUI and CLI.

use std::path::Path;

use super::{KResult, KnowledgeStore};

pub fn run(workspace: &Path, command: &str, rest: &str) -> KResult<String> {
    let mut store = KnowledgeStore::open(workspace)?;
    let (sub, arg) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let arg = arg.trim();
    match command {
        "/index" => {
            if sub == "rebuild" {
                store.clear_index()?;
            }
            let stats = store.refresh()?;
            let (files, symbols, chunks) = store.index_counts()?;
            Ok(format!(
                "Index: {files} files, {symbols} symbols, {chunks} chunks (parser v{}).\n{}",
                super::PARSER_VERSION,
                stats.describe()
            ))
        }
        "/memory" => {
            store.refresh()?;
            let user = KnowledgeStore::open_user().ok();
            match sub {
                "" | "list" => render(&store, user.as_ref(), |s| s.list_memories(15, false)),
                "stale" => render(&store, None, |s| s.list_memories(30, true)),
                "search" => render(&store, user.as_ref(), |s| s.search_memories(arg, 10)),
                "add" => {
                    let (scope, text) = match arg.strip_prefix("--user ") {
                        Some(t) => ("user", t.trim()),
                        None => ("project", arg),
                    };
                    let id = if scope == "user" {
                        user.ok_or("user memory unavailable")?.add_memory("user", "preference", text, &[])?
                    } else {
                        store.add_memory("project", "fact", text, &[])?
                    };
                    Ok(format!("Saved {} memory #{id}.", scope))
                }
                "forget" | "confirm" => {
                    let raw = arg.trim_start_matches('#');
                    let (target, id) = match raw.strip_prefix('u') {
                        Some(n) => (user.as_ref().ok_or("user memory unavailable")?, n),
                        None => (&store, raw.trim_start_matches('m')),
                    };
                    let id: i64 = id.parse().map_err(|_| format!("Usage: /memory {sub} <id>"))?;
                    let done = if sub == "forget" {
                        target.forget_memory(id)?
                    } else {
                        target.confirm_memory(id)?
                    };
                    Ok(if done {
                        format!("Memory {raw}: {}.", if sub == "forget" { "deleted" } else { "confirmed" })
                    } else {
                        format!("No memory {raw}.")
                    })
                }
                _ => Err("Usage: /memory [search <q>|add [--user] <text>|forget <id>|confirm <id>|stale]".into()),
            }
        }
        "/tasks" => {
            let tasks = store.recent_tasks(10)?;
            if tasks.is_empty() {
                return Ok("No tasks recorded yet.".into());
            }
            Ok(tasks
                .iter()
                .map(|t| {
                    let mut line = format!("#{} [{}] {}", t.id, if t.active { "active" } else { &t.status }, t.goal);
                    if !t.plan.is_empty() {
                        line.push_str(&format!("\n    plan: {}", t.render_plan()));
                    }
                    if !t.files.is_empty() {
                        line.push_str(&format!("\n    files: {}", t.files.join(", ")));
                    }
                    line
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "/goal" => match rest.trim() {
            "" => Ok(match store.active_task()? {
                Some(t) => format!("Active goal #{}: {}\n{}", t.id, t.goal, t.render_plan()),
                None => "No active goal. Use /goal <text>.".into(),
            }),
            "done" | "clear" => {
                let status = if rest.trim() == "done" { "done" } else { "abandoned" };
                match store.update_plan(None, &[], Some(status)) {
                    Ok(t) => Ok(format!("Goal #{} marked {status}.", t.id)),
                    Err(_) => Ok("No active goal.".into()),
                }
            }
            goal => Ok(format!("Active goal #{}: {goal}", store.start_task(goal)?)),
        },
        other => Err(format!("Unknown knowledge command {other}")),
    }
}

fn render(
    store: &KnowledgeStore,
    user: Option<&KnowledgeStore>,
    f: impl Fn(&KnowledgeStore) -> KResult<Vec<super::Memory>>,
) -> KResult<String> {
    let mut all = f(store)?;
    if let Some(u) = user {
        all.extend(f(u)?);
    }
    if all.is_empty() {
        return Ok("No memories.".into());
    }
    Ok(all.iter().map(|m| m.render()).collect::<Vec<_>>().join("\n"))
}
