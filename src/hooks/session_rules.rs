//! The session-rules block for SessionStart: role from the origin, text from
//! `.bosun/session-rules.toml` (spec §4). No GitHub call.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::git_out;
use crate::ctx::Origin;

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

const ORCHESTRATOR: &str = "Delegate work to child sessions with `bosun spawn` (models from `[models]`); archive each once its result is in. When context runs long: /compact or start fresh, and say which and why.";
const WORKER: &str = "Use superpowers (TDD) and caveman, with `[models]` subagents. Report in at most 10 lines. Spawn no sessions. At the end record spend: `bosun budget record --help`. When context runs long: compact or start fresh (handoff notes first), and say which.";
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

pub const START: &str = "<!-- bosun:session-rules:start -->";
pub const END: &str = "<!-- bosun:session-rules:end -->";

/// The marked Markdown block `bosun rules --write` keeps in CLAUDE.md or AGENTS.md, for
/// sessions whose hooks do not load (Claude in Slack).
pub fn markdown(r: &Rules) -> String {
    format!(
        "{START}\n## Session rules (bosun)\n\nRendered by `bosun rules --write` from \
         `.bosun/session-rules.toml`; edit that file, not this block. Hooks do not load \
         everywhere (Claude in Slack), so follow these here too.\n\n\
         - Orchestrator (a session not spawned by another): {}\n\
         - Worker (a spawned session): {}\n\
         - Everyone: {}\n\
         - No `bosun` on PATH: run `npx -y @keelstackhq/bosun@$(cat .bosun/version) <command>`.\n{END}\n",
        r.orchestrator, r.worker, r.all
    )
}

/// `text` with its marked block replaced by `block`, or `block` appended after a blank line.
/// Markers count only as whole lines (CRLF too). `None` when they are not one start followed by
/// one end (nothing to trust).
pub fn splice_block(text: &str, block: &str) -> Option<String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let at =
        |m: &str| -> Vec<usize> { (0..lines.len()).filter(|i| lines[*i].trim() == m).collect() };
    match (at(START).as_slice(), at(END).as_slice()) {
        ([], []) if text.trim().is_empty() => Some(block.to_string()),
        ([], []) => Some(format!("{}\n\n{block}", text.trim_end())),
        ([a], [b]) if a < b => Some(format!(
            "{}{block}{}",
            lines[..*a].concat(),
            lines[b + 1..].concat()
        )),
        _ => None,
    }
}

/// The rules from `<root>/.bosun/session-rules.toml` (defaults when absent), with a note when
/// the file is ignored.
pub fn load(root: Option<&Path>) -> (Rules, String) {
    let mut note = String::new();
    let mut rules = Rules::default();
    if let Some(file) = root.map(|r| r.join(".bosun/session-rules.toml"))
        && file.is_file()
    {
        match std::fs::read_to_string(&file)
            .map_err(|e| e.to_string())
            .and_then(|t| parse(&t))
        {
            Ok(r) => rules = r,
            Err(why) => {
                eprintln!("bosun: warning: .bosun/session-rules.toml: {why}");
                note = format!(" (.bosun/session-rules.toml ignored, defaults used: {why})");
            }
        }
    }
    (rules, note)
}

pub fn rules_context(dir: Option<&Path>) -> String {
    let origin = Origin::resolve(
        std::env::var("BOSUN_ORIGIN").ok().as_deref(),
        std::env::var("CLAUDE_CODE_CHILD_SESSION").ok().as_deref(),
    )
    .unwrap_or_else(|_| {
        // An invalid BOSUN_ORIGIN falls back to the child-session marker alone.
        Origin::resolve(
            None,
            std::env::var("CLAUDE_CODE_CHILD_SESSION").ok().as_deref(),
        )
        .unwrap_or(Origin::Local)
    });
    let role = role(origin.as_str());
    let dir = dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok());
    let root = dir.map(|d| git_out(&d, &["rev-parse", "--show-toplevel"]).map_or(d, PathBuf::from));
    let (rules, note) = load(root.as_deref());
    format!("{}{note}", block(&rules, role))
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
    fn splice_replaces_or_appends_the_block() {
        let b = format!("{START}\nnew\n{END}\n");
        assert_eq!(splice_block("", &b).unwrap(), b);
        assert_eq!(splice_block("a\n", &b).unwrap(), format!("a\n\n{b}"));
        let old = format!("a\n{START}\nold\n{END}\nz\n");
        assert_eq!(splice_block(&old, &b).unwrap(), format!("a\n{b}z\n"));
        assert!(splice_block(&format!("{END}\n{START}\n"), &b).is_none());
        assert!(splice_block(&format!("{START}\n{START}\n{END}"), &b).is_none());
        // CRLF settles; a marker quoted inside prose is no marker.
        let crlf = format!("a\r\n{START}\r\nold\r\n{END}\r\nz\r\n");
        assert_eq!(splice_block(&crlf, &b).unwrap(), format!("a\r\n{b}z\r\n"));
        let prose = format!("see `{START}`\n");
        assert_eq!(
            splice_block(&prose, &b).unwrap(),
            format!("see `{START}`\n\n{b}")
        );
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
