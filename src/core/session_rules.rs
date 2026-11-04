//! Session rules: the text SessionStart tells a session, by role (spec §4). Pure.

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Orchestrator,
    Worker,
}

/// A spawned session is a worker; every other origin (`Origin::as_str`) is an orchestrator.
pub fn role(origin: &str) -> Role {
    match origin {
        "spawned" => Role::Worker,
        _ => Role::Orchestrator,
    }
}

const ORCHESTRATOR: &str = "Split work into dedicated sessions (`bosun spawn`, models from `[models]`). Archive each child once its `result` is in. When context runs long, pick: /compact or start fresh (relay, spec §5.2); tell the owner which and why.";
const WORKER: &str = "Use superpowers (brainstorm, then plan; subagent-driven-development, TDD) and caveman, with `[models]` subagents. Report in at most 10 lines. Spawn no sessions. When context runs long, pick: compact or start fresh (write handoff notes first); say which.";
const ALL: &str = "Caveman style everywhere, chat with the owner included. Max token efficiency: write only what is needed.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rules {
    pub orchestrator: String,
    pub worker: String,
    pub all: String,
}

impl Default for Rules {
    fn default() -> Rules {
        Rules {
            orchestrator: ORCHESTRATOR.into(),
            worker: WORKER.into(),
            all: ALL.into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    orchestrator: Option<String>,
    worker: Option<String>,
    all: Option<String>,
}

/// Parse `.bosun/session-rules.toml`; a missing key keeps its default, an unknown key is an error.
pub fn parse(text: &str) -> Result<Rules, String> {
    let raw: Raw = toml::from_str(text).map_err(|e| e.to_string())?;
    let d = Rules::default();
    Ok(Rules {
        orchestrator: raw.orchestrator.unwrap_or(d.orchestrator),
        worker: raw.worker.unwrap_or(d.worker),
        all: raw.all.unwrap_or(d.all),
    })
}

pub fn block(rules: &Rules, role: Role) -> String {
    let (name, text) = match role {
        Role::Orchestrator => ("orchestrator", &rules.orchestrator),
        Role::Worker => ("worker", &rules.worker),
    };
    format!("Session rules ({name}): {text} {}", rules.all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawned_is_worker_others_orchestrator() {
        assert_eq!(role("spawned"), Role::Worker);
        for o in ["local", "slack", "routine"] {
            assert_eq!(role(o), Role::Orchestrator);
        }
    }

    #[test]
    fn parse_overrides_per_key_and_keeps_defaults() {
        let r = parse("worker = \"w\"").unwrap();
        assert_eq!(r.worker, "w");
        assert_eq!(r.orchestrator, Rules::default().orchestrator);
        assert_eq!(r.all, Rules::default().all);
        assert_eq!(parse("").unwrap(), Rules::default());
    }

    #[test]
    fn parse_rejects_unknown_keys_and_bad_toml() {
        assert!(parse("wrker = \"x\"").is_err());
        assert!(parse("worker = ").is_err());
    }

    #[test]
    fn block_joins_role_text_and_all() {
        let r = Rules {
            orchestrator: "o".into(),
            worker: "w".into(),
            all: "a".into(),
        };
        assert_eq!(block(&r, Role::Worker), "Session rules (worker): w a");
        assert_eq!(
            block(&r, Role::Orchestrator),
            "Session rules (orchestrator): o a"
        );
        assert!(block(&Rules::default(), Role::Worker).contains("Spawn no sessions."));
    }
}
