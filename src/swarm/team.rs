//! Team modes: competitive (independent candidates ranked by objective
//! verification), collaborative (one agent per area, in dependency order) and
//! Red Eclipse (red team attack paths, blue team fixes, verified rounds).

use std::time::Duration;

use serde_json::Value;

use super::*;
use crate::swarm::domain::COLLABORATIVE_SPECIALISTS;
use crate::swarm::workcopy::{self, FileDelta, Verification};
use crate::tools::policy::{self, Decision};
use crate::tools::sandbox::SandboxMode;
use crate::tools::ToolRisk;

const PLANNER: SwarmAgentRole = SwarmAgentRole {
    id: "planner",
    name: "Project Planner",
    badge: "PLAN",
    summary: "Splits the project into parts per area",
    system_directive: "\
Role: Project planner (Collaborative Swarm).\n\
Inspect the workspace with your read-only tools, then split the request into parts owned by these areas: database, backend, frontend, security, testing.\n\
Include only the areas the request really needs; every skipped area saves a full agent run.\n\
Each task must be concrete: the files or modules to touch and the contract the other areas rely on.\n\
Reply with JSON only, no prose: {\"parts\":[{\"area\":\"backend\",\"task\":\"...\"}]}",
};

fn clip(text: &str, max: usize) -> String {
    let t = crate::tools::truncate_str(text.trim(), max);
    if t.len() < text.trim().len() {
        format!("{t}…")
    } else {
        t.to_string()
    }
}

fn status(
    tx: &mpsc::UnboundedSender<SwarmEvent>,
    role: SwarmAgentRole,
    name: &str,
    status: SwarmAgentStatus,
) {
    let _ = tx.send(SwarmEvent::WorkerStatus {
        agent_id: role.id.to_string(),
        agent_name: name.to_string(),
        badge: role.badge.to_string(),
        status,
    });
}

fn phase(tx: &mpsc::UnboundedSender<SwarmEvent>, text: impl Into<String>) {
    let _ = tx.send(SwarmEvent::Phase(text.into()));
}

fn area_id(area: &str) -> Option<&'static str> {
    Some(match area.trim().to_ascii_lowercase().as_str() {
        "database" | "db" | "data" | "schema" | "storage" => "database",
        "backend" | "api" | "server" | "service" | "services" => "backend",
        "frontend" | "ui" | "client" | "web" => "frontend",
        "security" | "sec" | "hardening" => "security",
        "testing" | "test" | "tests" | "qa" => "testing",
        _ => return None,
    })
}

pub(super) fn parse_plan(text: &str) -> Vec<(&'static str, String)> {
    let (Some(start), Some(end)) = (text.find(['{', '[']), text.rfind(['}', ']'])) else {
        return Vec::new();
    };
    if end < start {
        return Vec::new();
    }
    let Ok(v) = serde_json::from_str::<Value>(&text[start..=end]) else {
        return Vec::new();
    };
    let items = v
        .get("parts")
        .and_then(Value::as_array)
        .or_else(|| v.as_array())
        .cloned()
        .unwrap_or_default();
    items
        .iter()
        .filter_map(|item| {
            let area = item.get("area").or_else(|| item.get("role"))?.as_str()?;
            let task = item.get("task")?.as_str()?.trim();
            (!task.is_empty()).then(|| Some((area_id(area)?, task.to_string())))?
        })
        .collect()
}

fn verification_line(cmd: &str, v: &Verification) -> String {
    let verdict = if v.passed { "PASSED" } else { "FAILED" };
    let mut line = format!("`{cmd}` {verdict} ({:.1}s)", v.secs);
    if !v.passed && !v.summary.is_empty() {
        for l in v
            .summary
            .lines()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            line.push_str(&format!("\n     {l}"));
        }
    }
    line
}

impl SwarmEngine {
    /// Same decision as for a direct tool call: sandbox policy first, then approval.
    async fn authorize(
        &self,
        event_tx: &mpsc::UnboundedSender<SwarmEvent>,
        tool_name: &str,
        risk: ToolRisk,
        summary: String,
        diff: Option<String>,
    ) -> Result<(), String> {
        match policy::evaluate(tool_name, risk, &self.tool_ctx, self.options.approval_mode) {
            Decision::Allow => Ok(()),
            Decision::Deny(reason) => Err(reason),
            Decision::Ask { note } => {
                let summary = match note {
                    Some(note) => format!("{summary} [{note}]"),
                    None => summary,
                };
                let (tx, rx) = oneshot::channel();
                let _ = event_tx.send(SwarmEvent::ToolApprovalRequested {
                    call_id: format!("swarm-{tool_name}"),
                    tool_name: tool_name.to_string(),
                    preview: ToolPreview { summary, diff },
                    respond_to: tx,
                });
                if rx.await.unwrap_or(false) {
                    Ok(())
                } else {
                    Err("denied by the user".into())
                }
            }
        }
    }

    fn verify_command(&self) -> Option<String> {
        self.options
            .verify_command
            .clone()
            .filter(|c| !c.trim().is_empty())
            .or_else(|| workcopy::detect_verify_command(&self.tool_ctx.workspace_root))
    }

    fn verify_timeout(&self) -> Duration {
        Duration::from_secs(self.options.verify_timeout_secs.max(1))
    }

    fn agent_name(&self, idx: usize, role: SwarmAgentRole) -> String {
        if self.worker_providers.is_empty() {
            role.name.to_string()
        } else {
            format!("{} [{}]", role.name, self.worker_provider(idx).model())
        }
    }

    /// Runs `roles` concurrently, one tool context each.
    #[allow(clippy::too_many_arguments)]
    async fn run_parallel(
        &self,
        session: &mut Session,
        roles: &[SwarmAgentRole],
        tools: Arc<ToolRegistry>,
        ctxs: Vec<ToolContext>,
        approval_mode: ApprovalMode,
        scope: Option<String>,
        prompt: String,
        event_tx: &mpsc::UnboundedSender<SwarmEvent>,
    ) -> Vec<WorkerOutcome> {
        let base_prompt = build_system_prompt(&self.tool_ctx.workspace_root);
        let prior = summarize_recent_history(session);
        let mixed = !self.worker_providers.is_empty();
        let handles: Vec<_> = roles
            .iter()
            .copied()
            .zip(ctxs)
            .enumerate()
            .map(|(idx, (role, ctx))| {
                let mut options = self.options.clone();
                options.approval_mode = approval_mode;
                tokio::spawn(run_worker(
                    role,
                    mixed,
                    self.worker_provider(idx),
                    Arc::clone(&tools),
                    ctx,
                    options,
                    base_prompt.clone(),
                    prior.clone(),
                    scope.clone(),
                    prompt.clone(),
                    event_tx.clone(),
                ))
            })
            .collect();
        let mut outcomes = Vec::with_capacity(roles.len());
        for (role, res) in roles.iter().copied().zip(join_all(handles).await) {
            match res {
                Ok(outcome) => {
                    session.add_usage(&outcome.usage);
                    outcomes.push(outcome);
                }
                Err(err) => {
                    let msg = format!("Worker task panicked: {err}");
                    status(
                        event_tx,
                        role,
                        role.name,
                        SwarmAgentStatus::Failed(msg.clone()),
                    );
                    outcomes.push(WorkerOutcome {
                        role,
                        report: String::new(),
                        usage: TokenUsage::default(),
                        error: Some(msg),
                    });
                }
            }
        }
        outcomes
    }

    /// One agent with full tools on the real workspace, under the user's approval mode.
    async fn run_sequential(
        &self,
        session: &mut Session,
        idx: usize,
        role: SwarmAgentRole,
        prompt: String,
        event_tx: &mpsc::UnboundedSender<SwarmEvent>,
    ) -> (String, Option<String>) {
        let name = self.agent_name(idx, role);
        status(
            event_tx,
            role,
            &name,
            SwarmAgentStatus::Running { active_tool: None },
        );
        let base = build_system_prompt(&self.tool_ctx.workspace_root);
        let engine = AgentEngine::with_system_prompt(
            self.worker_provider(idx),
            Arc::clone(&self.full_tools),
            self.tool_ctx.clone(),
            AgentOptions {
                max_tokens: self.options.max_tokens,
                temperature: self.options.temperature,
                max_tool_iterations: self.options.lead_max_iterations,
                approval_mode: self.options.approval_mode,
            },
            format!("{base}\n\n{}", role.system_directive),
        );
        let mut role_session = Session::new();
        let (report, error) = drive_agent(
            engine,
            &mut role_session,
            prompt,
            event_tx,
            Some((role, name.clone())),
            String::new(),
        )
        .await;
        let usage = role_session.total_usage().clone();
        session.add_usage(&usage);
        match &error {
            Some(err) => status(event_tx, role, &name, SwarmAgentStatus::Failed(err.clone())),
            None => {
                status(event_tx, role, &name, SwarmAgentStatus::Done);
                let _ = event_tx.send(SwarmEvent::WorkerReport {
                    agent_id: role.id.to_string(),
                    agent_name: name,
                    badge: role.badge.to_string(),
                    content: report.clone(),
                    usage: Some(usage),
                });
            }
        }
        (report, error)
    }

    /// Saver mode returns the deterministic report; otherwise the lead model writes it.
    async fn finish_report(
        &self,
        session: &mut Session,
        user_input: &str,
        lead_context: String,
        report: String,
        event_tx: &mpsc::UnboundedSender<SwarmEvent>,
    ) {
        let lead_badge = self.domain.lead_badge().to_string();
        let _ = event_tx.send(SwarmEvent::Synthesizing {
            lead_badge: lead_badge.clone(),
        });
        if self.options.saver {
            session.push_user(user_input.to_string());
            session.push_assistant(report.clone(), Vec::new());
            let _ = event_tx.send(SwarmEvent::SynthesisFinished {
                lead_badge,
                content: report,
                usage: None,
            });
        } else {
            self.run_lead(session, format!("{lead_context}\n\n{report}"), event_tx)
                .await;
        }
    }

    pub(super) async fn run_competitive(
        &self,
        session: &mut Session,
        user_input: String,
        scope: Option<String>,
        event_tx: &mpsc::UnboundedSender<SwarmEvent>,
    ) {
        if self.tool_ctx.sandbox.mode == SandboxMode::ReadOnly {
            let _ = event_tx.send(SwarmEvent::Error(
                "Competitive swarm needs workspace-write: candidates edit isolated copies of the workspace.".into(),
            ));
            return;
        }
        let roles = self.domain.specialists();
        let ws = self.tool_ctx.workspace_root.clone();
        phase(
            event_tx,
            format!(
                "Creating {} isolated copies of the workspace...",
                roles.len()
            ),
        );
        let snapshot_ws = ws.clone();
        let n = roles.len();
        let copies = tokio::task::spawn_blocking(move || {
            (0..n)
                .map(|_| workcopy::snapshot(&snapshot_ws))
                .collect::<std::io::Result<Vec<_>>>()
        })
        .await;
        let copies = match copies {
            Ok(Ok(copies)) => copies,
            Ok(Err(err)) => {
                let _ = event_tx.send(SwarmEvent::Error(format!(
                    "Could not create isolated copies: {err}"
                )));
                return;
            }
            Err(err) => {
                let _ = event_tx.send(SwarmEvent::Error(format!(
                    "Could not create isolated copies: {err}"
                )));
                return;
            }
        };
        let ctxs: Vec<ToolContext> = copies
            .iter()
            .map(|c| {
                let mut ctx = self.tool_ctx.clone();
                ctx.workspace_root = c.root.clone();
                ctx
            })
            .collect();

        let outcomes = self
            .run_parallel(
                session,
                roles,
                Arc::new(ToolRegistry::isolated_edit_registry()),
                ctxs.clone(),
                ApprovalMode::AutoAll,
                scope,
                user_input.clone(),
                event_tx,
            )
            .await;

        let roots: Vec<_> = copies.iter().map(|c| c.root.clone()).collect();
        let diff_ws = ws.clone();
        let deltas: Vec<Vec<FileDelta>> = tokio::task::spawn_blocking(move || {
            roots
                .iter()
                .map(|r| workcopy::changes(&diff_ws, r).unwrap_or_default())
                .collect()
        })
        .await
        .unwrap_or_default();

        let verify_cmd = self.verify_command();
        let mut results: Vec<Option<Verification>> = vec![None; n];
        let verify_note = match &verify_cmd {
            None => "No verification command found (set [swarm] verify); candidates ranked by change size only.".to_string(),
            Some(cmd) => {
                let ask = format!("Verify {n} candidates with `{cmd}` in their isolated copies");
                match self.authorize(event_tx, "run_command", ToolRisk::ShellExec, ask, None).await {
                    Err(reason) => format!("Verification skipped: {reason}."),
                    Ok(()) => {
                        let target = std::env::temp_dir().join("ecliven-swarm-target");
                        for (i, role) in roles.iter().copied().enumerate() {
                            if deltas.get(i).is_none_or(|d| d.is_empty()) {
                                continue;
                            }
                            let name = self.agent_name(i, role);
                            status(event_tx, role, &name, SwarmAgentStatus::Running {
                                active_tool: Some(format!("verify: {cmd}")),
                            });
                            let v = workcopy::verify(&ctxs[i], &ctxs[i].workspace_root, cmd, self.verify_timeout(), Some(&target)).await;
                            let st = if v.passed {
                                SwarmAgentStatus::Done
                            } else {
                                SwarmAgentStatus::Failed("verification failed".into())
                            };
                            status(event_tx, role, &name, st);
                            results[i] = Some(v);
                        }
                        format!("Verification: `{cmd}`")
                    }
                }
            }
        };

        struct Entry {
            idx: usize,
            diff: String,
            lines: usize,
            files: usize,
            passed: Option<bool>,
        }
        let mut ranked: Vec<Entry> = (0..n)
            .map(|idx| {
                let d = deltas.get(idx).map(Vec::as_slice).unwrap_or_default();
                let diff = workcopy::render_diff(d);
                Entry {
                    idx,
                    lines: workcopy::changed_lines(&diff),
                    diff,
                    files: d.len(),
                    passed: results[idx].as_ref().map(|v| v.passed),
                }
            })
            .collect();
        ranked.sort_by_key(|e| {
            let verdict = match e.passed {
                Some(true) => 0,
                None => 1,
                Some(false) => 2,
            };
            (e.files == 0, verdict, e.lines, e.idx)
        });

        let mut board = format!("Scoreboard. {verify_note}\n");
        for (pos, e) in ranked.iter().enumerate() {
            let role = roles[e.idx];
            let name = self.agent_name(e.idx, role);
            let verdict = match (&results[e.idx], &verify_cmd) {
                _ if e.files == 0 => "no changes".to_string(),
                (Some(v), Some(cmd)) => verification_line(cmd, v),
                _ => "not verified".to_string(),
            };
            let failed = outcomes
                .get(e.idx)
                .and_then(|o| o.error.as_deref())
                .map(|err| format!(" · agent error: {}", clip(err, 120)))
                .unwrap_or_default();
            board.push_str(&format!(
                "{}. [{}] {name}: {verdict} · {} files, {} lines{failed}\n",
                pos + 1,
                role.badge,
                e.files,
                e.lines
            ));
        }

        let winner = ranked
            .first()
            .filter(|e| e.files > 0 && e.passed != Some(false));
        let applied = match winner {
            None => "No candidate qualified: nothing was applied to the workspace.".to_string(),
            Some(w) => {
                let badge = roles[w.idx].badge;
                let ask = format!("Apply winning candidate [{badge}] ({} files)", w.files);
                match self
                    .authorize(
                        event_tx,
                        "apply_patch",
                        ToolRisk::FileWrite,
                        ask,
                        Some(clip(&w.diff, 30_000)),
                    )
                    .await
                {
                    Err(reason) => format!("Winner [{badge}] was not applied: {reason}."),
                    Ok(()) => match workcopy::apply(&self.tool_ctx, &deltas[w.idx]).await {
                        Ok(list) => format!(
                            "Applied winner [{badge}] to the workspace:\n{}",
                            list.join("\n")
                        ),
                        Err(err) => format!("Winner [{badge}] was not applied: {err}"),
                    },
                }
            }
        };
        phase(event_tx, format!("{board}\n{applied}"));

        let reports: Vec<String> = outcomes
            .iter()
            .map(|o| {
                format!(
                    "--- [{}] {} ---\n{}",
                    o.role.badge,
                    o.role.name,
                    clip(&o.report, 1500)
                )
            })
            .collect();
        let winner_diff = winner.map(|w| clip(&w.diff, 6000)).unwrap_or_default();
        let context = format!(
            "Original request:\n{user_input}\n\nCandidate reports:\n{}\n\nWinning diff:\n{winner_diff}",
            reports.join("\n")
        );
        drop(copies);
        self.finish_report(
            session,
            &user_input,
            context,
            format!("{board}\n{applied}"),
            event_tx,
        )
        .await;
    }

    pub(super) async fn run_collaborative(
        &self,
        session: &mut Session,
        user_input: String,
        scope: Option<String>,
        event_tx: &mpsc::UnboundedSender<SwarmEvent>,
    ) {
        phase(
            event_tx,
            "Planning: splitting the project into parts per area...",
        );
        let plan = self
            .run_parallel(
                session,
                &[PLANNER],
                Arc::clone(&self.read_only_tools),
                vec![self.tool_ctx.clone()],
                ApprovalMode::Ask,
                scope.clone(),
                user_input.clone(),
                event_tx,
            )
            .await;
        let mut parts = parse_plan(plan.first().map(|o| o.report.as_str()).unwrap_or(""));
        if parts.is_empty() {
            phase(
                event_tx,
                "The planner returned no usable plan: backend and testing take the whole task.",
            );
            parts = vec![
                ("backend", user_input.clone()),
                ("testing", format!("Test and verify: {user_input}")),
            ];
        }
        let plan_text: Vec<String> = parts
            .iter()
            .map(|(area, task)| format!("- {area}: {}", clip(task, 200)))
            .collect();
        phase(event_tx, format!("Plan:\n{}", plan_text.join("\n")));

        let mut handoffs: Vec<(SwarmAgentRole, String)> = Vec::new();
        for (idx, role) in COLLABORATIVE_SPECIALISTS.iter().copied().enumerate() {
            let tasks: Vec<&str> = parts
                .iter()
                .filter(|(area, _)| *area == role.id)
                .map(|(_, task)| task.as_str())
                .collect();
            if tasks.is_empty() {
                status(event_tx, role, role.name, SwarmAgentStatus::Skipped);
                continue;
            }
            let mut prompt = format!(
                "Project goal:\n{user_input}\n\nYour part ({}):\n{}\n",
                role.name,
                tasks.join("\n")
            );
            if let Some(scope) = &scope {
                prompt.push_str(&format!("\nWork inside `{scope}`.\n"));
            }
            if !handoffs.is_empty() {
                prompt.push_str("\nHandoffs from the agents before you:\n");
                for (r, h) in &handoffs {
                    prompt.push_str(&format!(
                        "--- [{}] {} ---\n{}\n",
                        r.badge,
                        r.name,
                        clip(h, 2000)
                    ));
                }
            }
            let (report, error) = self
                .run_sequential(session, idx, role, prompt, event_tx)
                .await;
            handoffs.push((
                role,
                error.map(|e| format!("(failed: {e})")).unwrap_or(report),
            ));
        }

        let summary: Vec<String> = handoffs
            .iter()
            .map(|(r, h)| format!("[{}] {}: {}", r.badge, r.name, clip(h, 600)))
            .collect();
        let full: Vec<String> = handoffs
            .iter()
            .map(|(r, h)| format!("--- [{}] {} ---\n{}", r.badge, r.name, clip(h, 3000)))
            .collect();
        let context = format!(
            "Project goal:\n{user_input}\n\nHandoffs:\n{}",
            full.join("\n")
        );
        self.finish_report(
            session,
            &user_input,
            context,
            format!("Delivered per area:\n{}", summary.join("\n")),
            event_tx,
        )
        .await;
    }

    pub(super) async fn run_red_eclipse(
        &self,
        session: &mut Session,
        user_input: String,
        scope: Option<String>,
        event_tx: &mpsc::UnboundedSender<SwarmEvent>,
    ) {
        let roles = self.domain.specialists();
        let red: Vec<SwarmAgentRole> = roles
            .iter()
            .copied()
            .filter(|r| r.id.starts_with("red-"))
            .collect();
        let blue: Vec<(usize, SwarmAgentRole)> = roles
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, r)| r.id.starts_with("blue-"))
            .collect();
        let rounds = self.options.rounds.clamp(1, 3);
        let verify_cmd = self.verify_command();
        let mut verify_allowed: Option<Result<(), String>> = None;
        let mut ledger: Vec<String> = Vec::new();

        for round in 1..=rounds {
            phase(
                event_tx,
                format!("Red Eclipse · round {round}/{rounds}: red team maps attack paths"),
            );
            let mut red_prompt =
                format!("Round {round} of {rounds}. Review request:\n{user_input}");
            if !ledger.is_empty() {
                red_prompt.push_str(&format!(
                    "\n\nEarlier rounds (re-test the fixed paths, then look for new ones):\n{}",
                    ledger.join("\n")
                ));
            }
            let red_out = self
                .run_parallel(
                    session,
                    &red,
                    Arc::clone(&self.read_only_tools),
                    vec![self.tool_ctx.clone(); red.len()],
                    ApprovalMode::Ask,
                    scope.clone(),
                    red_prompt,
                    event_tx,
                )
                .await;
            let findings: Vec<String> = red_out
                .iter()
                .filter(|o| o.error.is_none() && !o.report.trim().is_empty())
                .map(|o| {
                    format!(
                        "--- [{}] {} ---\n{}",
                        o.role.badge,
                        o.role.name,
                        clip(&o.report, 4000)
                    )
                })
                .collect();
            if findings.is_empty() {
                phase(event_tx, "The red team reported nothing: stopping.");
                break;
            }
            let findings = findings.join("\n");

            phase(
                event_tx,
                format!("Red Eclipse · round {round}/{rounds}: blue team hardens"),
            );
            let mut blue_notes = String::new();
            for (idx, role) in &blue {
                let mut prompt = format!("Round {round} of {rounds}. Review request:\n{user_input}\n\nRed team findings:\n{findings}\n");
                if !blue_notes.is_empty() {
                    prompt.push_str(&format!(
                        "\nBlue team work earlier this round:\n{blue_notes}"
                    ));
                }
                let (report, error) = self
                    .run_sequential(session, *idx, *role, prompt, event_tx)
                    .await;
                let note = error.map(|e| format!("(failed: {e})")).unwrap_or(report);
                blue_notes.push_str(&format!(
                    "--- [{}] ---\n{}\n",
                    role.badge,
                    clip(&note, 3000)
                ));
            }

            let verification = match &verify_cmd {
                None => "no verification command (set [swarm] verify)".to_string(),
                Some(cmd) => {
                    if verify_allowed.is_none() {
                        let ask = format!("Run `{cmd}` after each Red Eclipse round");
                        verify_allowed = Some(
                            self.authorize(event_tx, "run_command", ToolRisk::ShellExec, ask, None)
                                .await,
                        );
                    }
                    match verify_allowed.as_ref() {
                        Some(Err(reason)) => format!("skipped ({reason})"),
                        _ => {
                            let ws = self.tool_ctx.workspace_root.clone();
                            let v = workcopy::verify(
                                &self.tool_ctx,
                                &ws,
                                cmd,
                                self.verify_timeout(),
                                None,
                            )
                            .await;
                            verification_line(cmd, &v)
                        }
                    }
                }
            };
            phase(
                event_tx,
                format!("Round {round} verification: {verification}"),
            );
            ledger.push(format!(
                "Round {round}\nRed:\n{}\nBlue:\n{}\nVerification: {verification}",
                clip(&findings, 1500),
                clip(&blue_notes, 1500)
            ));
        }

        let context = format!("Review request:\n{user_input}\n\nRound ledger:");
        self.finish_report(session, &user_input, context, ledger.join("\n\n"), event_tx)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{AiError, CompletionRequest, CompletionResponse, ToolCall};
    use crate::tools::sandbox::SandboxPolicy;
    use async_trait::async_trait;

    struct Scripted;

    #[async_trait]
    impl LlmProvider for Scripted {
        fn name(&self) -> &'static str {
            "scripted"
        }

        fn model(&self) -> &str {
            "test/model"
        }

        async fn complete(
            &self,
            request: &CompletionRequest,
            _delta_tx: Option<mpsc::UnboundedSender<String>>,
        ) -> Result<CompletionResponse, AiError> {
            let sys = &request.system_prompt;
            let first = request.messages.len() == 1;
            let write = |content: &str| CompletionResponse {
                tool_calls: vec![ToolCall {
                    id: "w1".into(),
                    name: "write_file".into(),
                    arguments: serde_json::json!({ "path": "out.txt", "content": content }),
                }],
                ..CompletionResponse::default()
            };
            let text = |t: &str| CompletionResponse {
                content: t.into(),
                ..CompletionResponse::default()
            };
            Ok(if sys.contains("Competing implementer A") && first {
                write("good")
            } else if sys.contains("Competing implementer B") && first {
                write("bad")
            } else if sys.contains("Project planner") {
                text(
                    r#"Plan: {"parts":[{"area":"api","task":"add endpoint"},{"area":"qa","task":"test it"}]}"#,
                )
            } else if sys.contains("Red team") {
                text("attack path: unchecked input in src/lib.rs:1")
            } else {
                text("handoff done")
            })
        }
    }

    fn engine(domain: SwarmDomain, dir: &std::path::Path, verify: Option<&str>) -> SwarmEngine {
        let ctx = ToolContext::new(dir.to_path_buf(), 10).with_sandbox(SandboxPolicy::new(
            SandboxMode::DangerFullAccess,
            true,
            Vec::new(),
        ));
        SwarmEngine::new(
            domain,
            Arc::new(Scripted),
            Arc::new(ToolRegistry::default_registry()),
            ctx,
            SwarmOptions {
                approval_mode: ApprovalMode::AutoAll,
                verify_command: verify.map(str::to_string),
                rounds: 1,
                saver: true,
                ..SwarmOptions::default()
            },
        )
    }

    async fn run(engine: SwarmEngine, prompt: &str) -> Vec<SwarmEvent> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut session = Session::new();
        engine.run_turn(&mut session, prompt.into(), tx).await;
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        events
    }

    #[cfg(unix)]
    fn phases(events: &[SwarmEvent]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                SwarmEvent::Phase(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn plan_parsing_accepts_prose_and_aliases() {
        let parts = parse_plan("Sure:\n{\"parts\":[{\"area\":\"DB\",\"task\":\"t1\"},{\"area\":\"ui\",\"task\":\"t2\"},{\"area\":\"marketing\",\"task\":\"x\"}]}");
        assert_eq!(
            parts,
            vec![
                ("database", "t1".to_string()),
                ("frontend", "t2".to_string())
            ]
        );
        assert!(parse_plan("no json here").is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn competitive_applies_only_the_candidate_that_passes_verification() {
        let ws = tempfile::TempDir::new().unwrap();
        std::fs::write(ws.path().join("README"), "x\n").unwrap();
        let events = run(
            engine(
                SwarmDomain::Competitive,
                ws.path(),
                Some("grep -q good out.txt"),
            ),
            "create out.txt",
        )
        .await;
        let board = phases(&events);
        assert!(
            board.contains("1. [A]") && board.contains("PASSED"),
            "{board}"
        );
        assert!(board.contains("[B]") && board.contains("FAILED"), "{board}");
        assert!(
            board.contains("[C]") && board.contains("no changes"),
            "{board}"
        );
        assert!(board.contains("Applied winner [A]"), "{board}");
        assert_eq!(
            std::fs::read_to_string(ws.path().join("out.txt")).unwrap(),
            "good"
        );
        assert!(events
            .iter()
            .any(|e| matches!(e, SwarmEvent::SynthesisFinished { .. })));
    }

    #[tokio::test]
    async fn competitive_requires_write_access() {
        let ws = tempfile::TempDir::new().unwrap();
        let mut e = engine(SwarmDomain::Competitive, ws.path(), None);
        e.tool_ctx.sandbox = SandboxPolicy::new(SandboxMode::ReadOnly, true, Vec::new());
        let events = run(e, "task").await;
        assert!(events
            .iter()
            .any(|e| matches!(e, SwarmEvent::Error(m) if m.contains("workspace-write"))));
    }

    #[tokio::test]
    async fn collaborative_runs_only_planned_areas_in_order() {
        let ws = tempfile::TempDir::new().unwrap();
        let events = run(
            engine(SwarmDomain::Collaborative, ws.path(), None),
            "build feature",
        )
        .await;
        let skipped: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                SwarmEvent::WorkerStatus {
                    agent_id,
                    status: SwarmAgentStatus::Skipped,
                    ..
                } => Some(agent_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(skipped, vec!["database", "frontend", "security"]);
        let reports: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                SwarmEvent::WorkerReport { badge, .. } => Some(badge.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(reports, vec!["PLAN", "API", "QA"]);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn red_eclipse_runs_red_then_blue_then_verifies() {
        let ws = tempfile::TempDir::new().unwrap();
        let events = run(
            engine(SwarmDomain::RedEclipse, ws.path(), Some("true")),
            "review auth",
        )
        .await;
        let reports: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                SwarmEvent::WorkerReport { badge, .. } => Some(badge.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(&reports[2..], ["BLUE/FIX", "BLUE/TEST"]);
        assert!(reports[..2].iter().all(|b| b.starts_with("RED/")));
        assert!(phases(&events).contains("Round 1 verification: `true` PASSED"));
    }
}
