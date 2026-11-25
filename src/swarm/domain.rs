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
            Self::Code => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Security(SecuritySubMode::Defense) => "security:defense",
            Self::Security(SecuritySubMode::ThreatModel) => "security:threat",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Code => "Swarm: Code",
            Self::Security(SecuritySubMode::Defense) => "Swarm: Security [Defensa]",
            Self::Security(SecuritySubMode::ThreatModel) => "Swarm: Security [Threat Model]",
        }
    }

    pub fn subtitle(&self) -> &'static str {
        match self {
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
            Self::Code => "CHIEF",
            Self::Security(SecuritySubMode::Defense) => "DEFENSE LEAD",
            Self::Security(SecuritySubMode::ThreatModel) => "SEC ARCHITECT",
        }
    }

    pub fn parse(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "code" | "codigo" | "dev" | "eng" => Some(Self::Code),
            "security" | "sec" | "cyber" | "ciberseguridad" | "cybersecurity"
            | "security-defense" | "defense" | "defensa" => Some(Self::security_defense()),
            "security-threat" | "threat" | "stride" | "hardening" => Some(Self::security_threat()),
            _ => None,
        }
    }

    pub fn specialists(&self) -> &'static [SwarmAgentRole] {
        match self {
            Self::Code => CODE_SPECIALISTS,
            Self::Security(SecuritySubMode::Defense) => SECURITY_DEFENSE_SPECIALISTS,
            Self::Security(SecuritySubMode::ThreatModel) => SECURITY_THREAT_SPECIALISTS,
        }
    }

    pub fn lead_directive(&self) -> &'static str {
        match self {
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
