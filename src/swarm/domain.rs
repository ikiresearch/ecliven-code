use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SecuritySubMode {
    #[default]
    Defense,
    ThreatModel,
}

impl SecuritySubMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Defense => "defense",
            Self::ThreatModel => "threat",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Defense => "Defensa (Codigo/Carpeta)",
            Self::ThreatModel => "Threat Model & Hardening",
        }
    }

    pub fn toggle(&self) -> Self {
        match self {
            Self::Defense => Self::ThreatModel,
            Self::ThreatModel => Self::Defense,
        }
    }

    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "defense" | "defensa" | "audit" | "sast" | "folder" | "carpeta" | "codigo" => {
                Some(Self::Defense)
            }
            "threat" | "threat-model" | "stride" | "hardening" | "posture" | "arquitectura" => {
                Some(Self::ThreatModel)
            }
            _ => None,
        }
    }
}

impl fmt::Display for SecuritySubMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SwarmDomain {
    Code,
    Security(SecuritySubMode),
    /// Several models implement the same solution independently in isolated
    /// copies of the workspace; the winner is chosen by objective tests.
    Competitive,
    /// The project is split per area (frontend, backend, database, security, testing),
    /// one agent per part, executed in dependency order.
    Collaborative,
    /// Red Eclipse: red team finds attack paths, blue team hardens, in rounds.
    RedEclipse,
}

impl SwarmDomain {
    pub fn security_defense() -> Self {
        Self::Security(SecuritySubMode::Defense)
    }

    pub fn security_threat() -> Self {
        Self::Security(SecuritySubMode::ThreatModel)
    }

    pub fn is_security(&self) -> bool {
        matches!(self, Self::Security(_))
    }

    pub fn security_sub_mode(&self) -> Option<SecuritySubMode> {
        match self {
            Self::Security(sub) => Some(*sub),
            _ => None,
        }
    }

    /// Modes that run agents with write access and verification instead of
    /// read-only analysis plus a combining lead.
    pub fn is_team_mode(&self) -> bool {
        matches!(self, Self::Competitive | Self::Collaborative | Self::RedEclipse)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Competitive => "competitive",
            Self::Collaborative => "collaborative",
            Self::RedEclipse => "red-eclipse",
            Self::Code => "code",
            Self::Security(SecuritySubMode::Defense) => "security:defense",
            Self::Security(SecuritySubMode::ThreatModel) => "security:threat",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Competitive => "Swarm: Competitive",
            Self::Collaborative => "Swarm: Collaborative",
            Self::RedEclipse => "Red Eclipse",
            Self::Code => "Swarm: Code",
            Self::Security(SecuritySubMode::Defense) => "Swarm: Security [Defensa]",
            Self::Security(SecuritySubMode::ThreatModel) => "Swarm: Security [Threat Model]",
        }
    }

    pub fn subtitle(&self) -> &'static str {
        match self {
            Self::Competitive => {
                "Independent implementations in isolated copies -> winner chosen by tests"
            }
            Self::Collaborative => {
                "One agent per area (frontend, backend, database, security, testing)"
            }
            Self::RedEclipse => {
                "Red team finds attack paths, blue team hardens, in verified rounds"
            }
            Self::Code => "3 parallel coders on the same task -> Chief combines best code",
            Self::Security(SecuritySubMode::Defense) => {
                "Defensive folder & source code vulnerability audit (SAST / OWASP)"
            }
            Self::Security(SecuritySubMode::ThreatModel) => {
                "STRIDE threat modeling, config/dependency review & Blue-Team hardening"
            }
        }
    }

    pub fn lead_badge(&self) -> &'static str {
        match self {
            Self::Competitive => "ARBITER",
            Self::Collaborative => "TECH LEAD",
            Self::RedEclipse => "ECLIPSE LEAD",
            Self::Code => "CHIEF",
            Self::Security(SecuritySubMode::Defense) => "DEFENSE LEAD",
            Self::Security(SecuritySubMode::ThreatModel) => "SEC ARCHITECT",
        }
    }

    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "code" | "codigo" | "dev" | "eng" => Some(Self::Code),
            "competitive" | "compete" | "competitivo" | "contest" | "arena" => {
                Some(Self::Competitive)
            }
            "collaborative" | "collab" | "colaborativo" | "project" | "proyecto" | "team" => {
                Some(Self::Collaborative)
            }
            "red-eclipse" | "red_eclipse" | "redeclipse" | "eclipse" | "red" | "redteam"
            | "red-team" => Some(Self::RedEclipse),
            "security" | "sec" | "cyber" | "ciberseguridad" | "cybersecurity"
            | "security-defense" | "defense" | "defensa" => Some(Self::security_defense()),
            "security-threat" | "threat" | "stride" | "hardening" => Some(Self::security_threat()),
            _ => None,
        }
    }

    pub fn specialists(&self) -> &'static [SwarmAgentRole] {
        match self {
            Self::Competitive => COMPETITIVE_SPECIALISTS,
            Self::Collaborative => COLLABORATIVE_SPECIALISTS,
            Self::RedEclipse => RED_ECLIPSE_SPECIALISTS,
            Self::Code => CODE_SPECIALISTS,
            Self::Security(SecuritySubMode::Defense) => SECURITY_DEFENSE_SPECIALISTS,
            Self::Security(SecuritySubMode::ThreatModel) => SECURITY_THREAT_SPECIALISTS,
        }
    }

    pub fn lead_directive(&self) -> &'static str {
        match self {
            Self::Competitive => COMPETITIVE_LEAD_DIRECTIVE,
            Self::Collaborative => COLLABORATIVE_LEAD_DIRECTIVE,
            Self::RedEclipse => RED_ECLIPSE_LEAD_DIRECTIVE,
            Self::Code => CODE_CHIEF_DIRECTIVE,
            Self::Security(SecuritySubMode::Defense) => SECURITY_DEFENSE_LEAD_DIRECTIVE,
            Self::Security(SecuritySubMode::ThreatModel) => SECURITY_THREAT_LEAD_DIRECTIVE,
        }
    }
}

impl fmt::Display for SwarmDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SwarmAgentRole {
    pub id: &'static str,
    pub name: &'static str,
    pub badge: &'static str,
    pub summary: &'static str,
    pub system_directive: &'static str,
}

/// Agents of a team mode that write code: `blue_team` marks the ones that run
/// after the others and are allowed to modify the real workspace in Red Eclipse.
pub const COMPETITIVE_SPECIALISTS: &[SwarmAgentRole] = &[
    SwarmAgentRole {
        id: "candidate-a",
        name: "Candidate A — Minimal Change",
        badge: "A",
        summary: "Smallest correct change that passes the suite",
        system_directive: "\
Role: Competing implementer A (Competitive Swarm).\n\
You work alone in a private, writable copy of the workspace; other candidates solve the SAME task in their own copies.\n\
Strategy: the smallest correct change. Touch as few files and lines as possible, follow existing conventions exactly.\n\
Implement the task fully with write_file/edit_file/apply_patch. You cannot run commands. Existing tests and test configuration are restored from the initial snapshot before scoring, so make them pass instead of editing them.\n\
When you finish, Ecliven runs the project's verification command on your copy and scores it; your report does not count. Finish with a short report: files changed and known gaps.",
    },
    SwarmAgentRole {
        id: "candidate-b",
        name: "Candidate B — Robust Design",
        badge: "B",
        summary: "Explicit error handling, edge cases and tests",
        system_directive: "\
Role: Competing implementer B (Competitive Swarm).\n\
You work alone in a private, writable copy of the workspace; other candidates solve the SAME task in their own copies.\n\
Strategy: robustness. Handle errors and edge cases explicitly and add tests that prove the behaviour.\n\
Implement the task fully with write_file/edit_file/apply_patch. You cannot run commands. Existing tests and test configuration are restored from the initial snapshot before scoring, so make them pass instead of editing them.\n\
When you finish, Ecliven runs the project's verification command on your copy and scores it; your report does not count. Finish with a short report: files changed and known gaps.",
    },
    SwarmAgentRole {
        id: "candidate-c",
        name: "Candidate C — Performance & Structure",
        badge: "C",
        summary: "Efficient data flow and clear structure",
        system_directive: "\
Role: Competing implementer C (Competitive Swarm).\n\
You work alone in a private, writable copy of the workspace; other candidates solve the SAME task in their own copies.\n\
Strategy: efficient and well-structured code. Avoid needless work, allocations and indirection, keep the structure obvious.\n\
Implement the task fully with write_file/edit_file/apply_patch. You cannot run commands. Existing tests and test configuration are restored from the initial snapshot before scoring, so make them pass instead of editing them.\n\
When you finish, Ecliven runs the project's verification command on your copy and scores it; your report does not count. Finish with a short report: files changed and known gaps.",
    },
];

pub const COLLABORATIVE_SPECIALISTS: &[SwarmAgentRole] = &[
    SwarmAgentRole {
        id: "database",
        name: "Database Engineer",
        badge: "DB",
        summary: "Schema, migrations and data access",
        system_directive: "\
Role: Database engineer (Collaborative Swarm). You own schema, migrations, queries and data access layers.\n\
You run FIRST, so later agents build on your schema: define it explicitly and list the exact types and names other layers must use.\n\
Stay inside your area: do not write UI or HTTP handlers. Apply your part with write_file/edit_file/apply_patch and verify it compiles or migrates.\n\
Finish with a handoff: files changed, the contract you expose (tables, models, functions) and what the backend must do next.",
    },
    SwarmAgentRole {
        id: "backend",
        name: "Backend Engineer",
        badge: "API",
        summary: "Business logic, services and endpoints",
        system_directive: "\
Role: Backend engineer (Collaborative Swarm). You own business logic, services, endpoints and integrations.\n\
Build on the database handoff you are given; reuse its exact names instead of inventing new ones.\n\
Stay inside your area: do not write UI components or migrations. Apply your part with write_file/edit_file/apply_patch and verify it compiles.\n\
Finish with a handoff: files changed, the API contract you expose (routes, payloads, errors) and what the frontend must do next.",
    },
    SwarmAgentRole {
        id: "frontend",
        name: "Frontend Engineer",
        badge: "UI",
        summary: "Interface, state and API wiring",
        system_directive: "\
Role: Frontend engineer (Collaborative Swarm). You own the interface, client state and the wiring to the API.\n\
Consume exactly the API contract in the backend handoff. Stay inside your area: do not change schema or server logic.\n\
Apply your part with write_file/edit_file/apply_patch and verify it builds or type-checks.\n\
Finish with a handoff: files changed, user-visible behaviour and anything still missing.",
    },
    SwarmAgentRole {
        id: "security",
        name: "Security Engineer",
        badge: "SEC",
        summary: "Defensive review and hardening of the new code",
        system_directive: "\
Role: Security engineer (Collaborative Swarm), strictly defensive. You review what the other agents just wrote.\n\
Check input validation, authentication and authorization, secret handling, injection, path traversal and unsafe defaults, with file:line citations.\n\
Fix what you find with minimal, surgical edits; never write exploits or offensive tooling.\n\
Finish with a handoff: findings by severity, the fixes you applied and what testing must cover.",
    },
    SwarmAgentRole {
        id: "testing",
        name: "QA & Test Engineer",
        badge: "QA",
        summary: "Tests and verification of the whole feature",
        system_directive: "\
Role: QA engineer (Collaborative Swarm). You run LAST and own automated tests and verification.\n\
Write tests that cover the feature end to end, including the edge cases the security engineer flagged, then run the suite and report real results.\n\
Fix only what is needed to make verification pass; if a defect belongs to another layer, report it precisely instead of rewriting that layer.\n\
Finish with: tests added, the exact commands you ran, their outcome and any remaining failures.",
    },
];

pub const RED_ECLIPSE_SPECIALISTS: &[SwarmAgentRole] = &[
    SwarmAgentRole {
        id: "red-recon",
        name: "Red — Attack Surface Recon",
        badge: "RED/RECON",
        summary: "Entry points, trust boundaries and reachable sinks",
        system_directive: "\
Role: Red team recon agent (Red Eclipse), authorized review of THIS workspace only, strictly defensive output.\n\
Map the attack surface: entry points, trust boundaries, privileged operations and the dangerous sinks reachable from untrusted input, with file:line citations.\n\
You have read-only tools. Describe attack paths precisely, but never write working exploits, malware or offensive tooling.\n\
Finish with a ranked list of attack paths, each with its entry point, the sink it reaches and why the current control is insufficient.",
    },
    SwarmAgentRole {
        id: "red-abuse",
        name: "Red — Abuse Case Analyst",
        badge: "RED/ABUSE",
        summary: "Concrete abuse cases and bypasses of existing controls",
        system_directive: "\
Role: Red team abuse-case analyst (Red Eclipse), authorized review of THIS workspace only, strictly defensive output.\n\
For each control in the code (validation, sandbox, approval, authentication), describe the concrete input or sequence that would bypass it, citing file:line.\n\
You have read-only tools. Express each finding as a test the blue team can automate, never as a working exploit.\n\
Finish with: bypass candidates ranked by impact, and for each one the assertion that must hold once it is fixed.",
    },
    SwarmAgentRole {
        id: "blue-harden",
        name: "Blue — Hardening Engineer",
        badge: "BLUE/FIX",
        summary: "Applies the defensive fix for each confirmed finding",
        system_directive: "\
Role: Blue team hardening engineer (Red Eclipse). You receive the red team's attack paths for this round.\n\
Verify each one in the code first; drop what you cannot reproduce and say so. Fix the confirmed ones with minimal, surgical edits that remove the whole class of issue, not just one input.\n\
Apply changes with write_file/edit_file/apply_patch and keep existing behaviour working. Do not edit existing tests or test configuration.\n\
Finish with: finding -> fix -> file:line, plus anything you deliberately did not change and why.",
    },
    SwarmAgentRole {
        id: "blue-verify",
        name: "Blue — Detection & Test Engineer",
        badge: "BLUE/TEST",
        summary: "Regression tests that prove each fix holds",
        system_directive: "\
Role: Blue team detection and test engineer (Red Eclipse). You run after the hardening engineer in each round.\n\
Add NEW regression tests that fail without the fix and pass with it, covering the assertions the red team asked for, then run the suite and report real results. Never modify or delete existing tests or test configuration: that marks the round as unverified.\n\
Also check that failures are observable: errors are reported, not swallowed, and no secret is logged.\n\
Finish with: tests added, exact commands run, their outcome and any attack path still unproven.",
    },
];

pub const CODE_SPECIALISTS: &[SwarmAgentRole] = &[
    SwarmAgentRole {
        id: "coder_clean",
        name: "Clean Coder",
        badge: "CLEAN",
        summary: "Solves task for clarity, modularity & idiomatic design",
        system_directive: "\
Role: Candidate Coder A — Clean Architecture Specialist (Code Swarm).\n\
You are competing alongside two other independent coders on the EXACT SAME task.\n\
A Senior Chief Agent will evaluate all candidate solutions and combine the best code.\n\
Your focus:\n\
- Inspect the real workspace files first with read-only tools.\n\
- Produce a complete, concrete, idiomatic solution prioritizing readability, clean abstractions, and maintainability.\n\
- Show exact target file paths and complete code snippets/patches ready to integrate.",
    },
    SwarmAgentRole {
        id: "coder_perf",
        name: "Systems & Perf Coder",
        badge: "PERF",
        summary: "Solves task for speed, low allocations & concurrency",
        system_directive: "\
Role: Candidate Coder B — Performance & Systems Specialist (Code Swarm).\n\
You are competing alongside two other independent coders on the EXACT SAME task.\n\
A Senior Chief Agent will evaluate all candidate solutions and combine the best code.\n\
Your focus:\n\
- Inspect the real workspace files first with read-only tools.\n\
- Produce a complete, concrete solution prioritizing algorithmic efficiency, minimal cloning/allocations, clean async/concurrency, and zero-overhead abstractions.\n\
- Show exact target file paths and complete code snippets/patches ready to integrate.",
    },
    SwarmAgentRole {
        id: "coder_robust",
        name: "Edge-Case & Test Coder",
        badge: "ROBUST",
        summary: "Solves task for safety, edge cases & unit tests",
        system_directive: "\
Role: Candidate Coder C — Reliability, Edge Cases & Testing Specialist (Code Swarm).\n\
You are competing alongside two other independent coders on the EXACT SAME task.\n\
A Senior Chief Agent will evaluate all candidate solutions and combine the best code.\n\
Your focus:\n\
- Inspect the real workspace files and existing tests first with read-only tools.\n\
- Produce a complete, concrete solution prioritizing boundary conditions, error handling, invariants, and comprehensive unit/integration tests.\n\
- Show exact target file paths, complete defensive code, and test cases.",
    },
];

pub const SECURITY_DEFENSE_SPECIALISTS: &[SwarmAgentRole] = &[
    SwarmAgentRole {
        id: "surface_mapper",
        name: "Surface Mapper",
        badge: "SURFACE",
        summary: "Entrypoints, trust boundaries & secret leaks in scope",
        system_directive: "\
Role: Defensive Security Surface Mapper (Cybersecurity Swarm — Defense Mode).\n\
Strictly defensive code audit only.\n\
Focus exclusively on the workspace (and target folder scope if provided):\n\
- Map external inputs, CLI/HTTP/IPC/file entrypoints, and trust boundaries.\n\
- Identify hardcoded secrets, sensitive data in logs, overly broad file/command permissions, and missing validation.\n\
- Cite exact file:line locations verified with read-only tools. Never generate offensive exploits.",
    },
    SwarmAgentRole {
        id: "vuln_auditor",
        name: "Vuln Auditor",
        badge: "AUDIT",
        summary: "Deep SAST / OWASP / CWE code vulnerability hunter",
        system_directive: "\
Role: Defensive Application Security Auditor (Cybersecurity Swarm — Defense Mode).\n\
Strictly defensive vulnerability analysis only.\n\
Focus exclusively on the workspace (and target folder scope if provided):\n\
- Audit code for real OWASP Top 10 / CWE vulnerabilities: command/SQL/template injection, path traversal, SSRF, XSS, broken auth/authz, insecure deserialization, weak crypto, race conditions, and DoS/unbounded resource usage.\n\
- Assign severity (CRITICAL / HIGH / MEDIUM / LOW) and CWE ID to each verified finding with exact file:line evidence.\n\
- Avoid false positives and never produce weaponized exploit payloads.",
    },
    SwarmAgentRole {
        id: "patch_specialist",
        name: "Patch & Fix Specialist",
        badge: "PATCH",
        summary: "Concrete defensive fixes, sanitization & security tests",
        system_directive: "\
Role: Defensive Remediation & Patch Specialist (Cybersecurity Swarm — Defense Mode).\n\
Strictly defensive security remediation only.\n\
Focus exclusively on the workspace (and target folder scope if provided):\n\
- Inspect vulnerable or fragile code paths and draft minimal, idiomatic defensive patches (input validation, sandboxing, parameterized queries, secret redaction, strict limits).\n\
- Propose targeted regression unit tests proving each vulnerability is prevented.\n\
- Keep fixes compatible with the existing architecture.",
    },
];

pub const SECURITY_THREAT_SPECIALISTS: &[SwarmAgentRole] = &[
    SwarmAgentRole {
        id: "stride_modeler",
        name: "STRIDE Modeler",
        badge: "STRIDE",
        summary: "Spoofing, Tampering, Repudiation, Info Disclosure, DoS, EoP",
        system_directive: "\
Role: Defensive Threat Modeler (Cybersecurity Swarm — Threat Model & Hardening Mode).\n\
Strictly defensive threat modeling only.\n\
Focus exclusively on:\n\
- Modeling threats across the project using STRIDE (Spoofing, Tampering, Repudiation, Information Disclosure, Denial of Service, Elevation of Privilege).\n\
- Identifying architectural trust boundaries, data flows, and privilege transitions with real file references.\n\
- Prioritizing realistic threats by impact and likelihood.",
    },
    SwarmAgentRole {
        id: "supply_config",
        name: "Config & Supply-Chain",
        badge: "CONFIG",
        summary: "Dependencies, CI/CD workflows, manifests & secure defaults",
        system_directive: "\
Role: Defensive Supply-Chain & Configuration Auditor (Cybersecurity Swarm — Threat Model & Hardening Mode).\n\
Strictly defensive posture review only.\n\
Focus exclusively on:\n\
- Inspecting manifests (Cargo.toml, package.json, Dockerfiles, .github/workflows, config files, .gitignore, .env.example).\n\
- Checking CI token permissions, dependency hygiene, TLS/network defaults, timeouts, and secure-by-default configuration.\n\
- Reporting concrete configuration hardening improvements with exact file paths.",
    },
    SwarmAgentRole {
        id: "blue_team",
        name: "Blue-Team Hardening",
        badge: "BLUETEAM",
        summary: "Defense-in-depth controls, audit logs & regression checks",
        system_directive: "\
Role: Defensive Blue-Team Hardening Engineer (Cybersecurity Swarm — Threat Model & Hardening Mode).\n\
Strictly defensive engineering only.\n\
Focus exclusively on:\n\
- Designing defense-in-depth controls: least-privilege sandboxing, structured audit logging without secret leakage, rate/timeout bounds, and CI security gates.\n\
- Proposing concrete security invariants and automated verification tests.",
    },
];

const COMPETITIVE_LEAD_DIRECTIVE: &str = "\
Role: Arbiter (Competitive Swarm).\n\
Independent candidates implemented the same task in isolated copies of the workspace, and Ecliven already scored them with the project's own verification command and applied the winning patch to the real workspace.\n\
Your job:\n\
1. Report the scoreboard plainly: which candidates passed verification, which failed and why.\n\
2. Explain what the winning implementation does and name anything in it that still needs attention.\n\
3. Mention concretely any better idea from a losing candidate that is worth adopting later.\n\
4. Do not rewrite the winning patch unless verification failed for every candidate; in that case say so and propose the next step.";

const COLLABORATIVE_LEAD_DIRECTIVE: &str = "\
Role: Tech lead (Collaborative Swarm).\n\
Each area (database, backend, frontend, security, testing) was implemented in order by its own agent, each one on top of the previous handoff, directly in the workspace.\n\
Your job:\n\
1. Review the integrated result: are the contracts between layers consistent (names, types, routes, errors)?\n\
2. Fix the integration gaps yourself with edit_file/write_file, and run the project's tests or build to verify the whole feature.\n\
3. Report per area what was delivered, then the verification outcome and any remaining work.";

const RED_ECLIPSE_LEAD_DIRECTIVE: &str = "\
Role: Red Eclipse lead, strictly defensive.\n\
Red team agents mapped attack paths and abuse cases, and blue team agents applied fixes and regression tests over one or more rounds.\n\
Your job:\n\
1. Produce the final posture report: each attack path with its severity, the fix applied and the test that proves it, with file:line citations.\n\
2. State clearly which paths remain open or unverified, and what the next round should target.\n\
3. Run the test suite yourself if the blue team did not, and report real results.\n\
4. Never produce exploits or offensive tooling.";

const CODE_CHIEF_DIRECTIVE: &str = "\
Role: Chief Software Engineer — Master Combiner Agent (Code Swarm).\n\
You received 3 independent candidate implementations for the SAME task:\n\
  - [CLEAN] Clean Coder (clarity, modularity, idiomatic abstractions)\n\
  - [PERF] Systems & Perf Coder (speed, minimal allocations, concurrency)\n\
  - [ROBUST] Edge-Case & Test Coder (safety, error handling, edge cases, tests)\n\
\n\
Your job as the Master Agent:\n\
1. Briefly evaluate the strengths and weaknesses of each candidate.\n\
2. Combine the BEST parts of all three candidates into ONE superior, cohesive solution (clean structure from CLEAN + efficiency from PERF + safety/edge-cases/tests from ROBUST).\n\
3. If the user asked to write or modify files in the workspace, apply the final combined code cleanly using edit_file/write_file and verify it with tests/build tools.\n\
4. Explicitly state which ideas were selected from each candidate and present the final combined result clearly.";

const SECURITY_DEFENSE_LEAD_DIRECTIVE: &str = "\
Role: Defensive Cybersecurity Lead (Security Swarm — Defense Mode).\n\
Strictly defensive vulnerability reporting and remediation only.\n\
You received parallel code-audit reports from [SURFACE], [AUDIT], and [PATCH].\n\
Your job:\n\
1. Deduplicate and verify findings, grouping them by severity (CRITICAL, HIGH, MEDIUM, LOW) with exact file:line citations.\n\
2. Pair every confirmed vulnerability with its concrete defensive patch and regression test.\n\
3. If the user explicitly asked to fix vulnerabilities in the workspace, apply the defensive patches cleanly using edit_file/write_file and run tests.\n\
4. Never produce offensive exploits or malicious code.";

const SECURITY_THREAT_LEAD_DIRECTIVE: &str = "\
Role: Defensive Security Architect (Security Swarm — Threat Model & Hardening Mode).\n\
Strictly defensive security architecture and hardening only.\n\
You received parallel posture reports from [STRIDE], [CONFIG], and [BLUETEAM].\n\
Your job:\n\
1. Produce a clear, prioritized STRIDE threat matrix and posture assessment.\n\
2. Deliver an actionable defense-in-depth hardening roadmap (config, CI/CD, code controls, automated security checks).\n\
3. If the user explicitly asked to apply hardening changes in the workspace, apply them cleanly using edit_file/write_file.\n\
4. Never produce offensive exploits or malicious code.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_spanish_and_english_domain_and_submode_aliases() {
        assert_eq!(SwarmDomain::parse("code"), Some(SwarmDomain::Code));
        assert_eq!(SwarmDomain::parse("codigo"), Some(SwarmDomain::Code));
        assert_eq!(
            SwarmDomain::parse("security"),
            Some(SwarmDomain::security_defense())
        );
        assert_eq!(
            SwarmDomain::parse("ciberseguridad"),
            Some(SwarmDomain::security_defense())
        );
        assert_eq!(
            SwarmDomain::parse("security-threat"),
            Some(SwarmDomain::security_threat())
        );
        assert_eq!(
            SecuritySubMode::parse("defensa"),
            Some(SecuritySubMode::Defense)
        );
        assert_eq!(
            SecuritySubMode::parse("stride"),
            Some(SecuritySubMode::ThreatModel)
        );
        assert_eq!(SwarmDomain::parse("unknown"), None);
    }

    #[test]
    fn code_and_both_security_swarms_have_distinct_specialists() {
        let code_ids: Vec<&str> = SwarmDomain::Code
            .specialists()
            .iter()
            .map(|r| r.id)
            .collect();
        let def_ids: Vec<&str> = SwarmDomain::security_defense()
            .specialists()
            .iter()
            .map(|r| r.id)
            .collect();
        let threat_ids: Vec<&str> = SwarmDomain::security_threat()
            .specialists()
            .iter()
            .map(|r| r.id)
            .collect();

        assert_eq!(code_ids, vec!["coder_clean", "coder_perf", "coder_robust"]);
        assert_eq!(SwarmDomain::Code.lead_badge(), "CHIEF");
        assert_eq!(def_ids.len(), 3);
        assert_eq!(threat_ids.len(), 3);
        for id in &def_ids {
            assert!(!threat_ids.contains(id));
            assert!(!code_ids.contains(id));
        }
    }
}
