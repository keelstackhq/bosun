//! The session-rules block for SessionStart: role from the origin, text from
//! `.bosun/session-rules.toml` (spec §4). No GitHub call.

use std::path::{Path, PathBuf};

use super::git_out;
use crate::core::session_rules::{self, Rules};
use crate::ctx::Origin;

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
    let role = session_rules::role(origin.as_str());
    let dir = dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok());
    let root = dir.map(|d| git_out(&d, &["rev-parse", "--show-toplevel"]).map_or(d, PathBuf::from));
    let mut note = String::new();
    let mut rules = Rules::default();
    if let Some(file) = root.map(|r| r.join(".bosun/session-rules.toml"))
        && file.is_file()
    {
        match std::fs::read_to_string(&file)
            .map_err(|e| e.to_string())
            .and_then(|t| session_rules::parse(&t))
        {
            Ok(r) => rules = r,
            Err(why) => {
                eprintln!("bosun: warning: .bosun/session-rules.toml: {why}");
                note = format!(" (.bosun/session-rules.toml ignored, defaults used: {why})");
            }
        }
    }
    format!("{}{note}", session_rules::block(&rules, role))
}
