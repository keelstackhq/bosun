//! `bosun setup-snippet`: print the bootstrap to paste once as a cloud environment's setup
//! script (spec D23). It runs each checkout's own docs/setup-script.sh, so a release never
//! needs a new paste. With `[session]` auto-compact keys it also prints their variables.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::config::{CONFIG_FILE, Session};
use crate::output::{BosunError, CmdResult, Outcome};

/// The bootstrap text; README.md shows it verbatim (a test checks).
pub const BOOTSTRAP: &str = include_str!("setup_bootstrap.sh");

/// One line the owner can send a client together with the script.
const CLIENT_MESSAGE: &str = "Paste this script once as the setup script of your Claude Code cloud environment (claude.ai/code > Environments > Setup script). You never change it again: each repo's own docs/setup-script.sh pins its tools.";

/// Where `cloud_env` goes: the environment's variables, not the setup script (its exports end
/// with its shell) and not settings.json `env` (cloud sessions skip it).
const ENV_PASTE: &str = "claude.ai/code > Environments > <environment> > Environment variables";

/// Cloud sessions set CLAUDE_AUTOCOMPACT_PCT_OVERRIDE themselves (Claude Code cloud docs).
const PCT_NOTE: &str = "autocompact_pct is for local sessions only: a cloud session sets CLAUDE_AUTOCOMPACT_PCT_OVERRIDE itself and ignores an environment value; use autocompact_window for cloud";

pub fn run(dir: Option<&Path>) -> CmdResult {
    let mut data = json!({
        "script": BOOTSTRAP,
        "paste_into": "claude.ai/code > Environments > <environment> > Setup script",
        "repo_script": "docs/setup-script.sh",
        "client_message": CLIENT_MESSAGE,
    });
    if let Some(a) = autocompact(&session(dir))? {
        data["autocompact"] = a;
    }
    Ok(Outcome::data(data))
}

/// `[session]` of the repo's bosun.toml, read alone: an unreadable file, bad TOML or another
/// section (from a newer bosun) never fails the snippet; an absent table is the default.
fn session(dir: Option<&Path>) -> Session {
    let Some(start) = dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
    else {
        return Session::default();
    };
    let root =
        crate::ctx::git_out(&start, &["rev-parse", "--show-toplevel"]).map_or(start, PathBuf::from);
    let text = std::fs::read_to_string(root.join(CONFIG_FILE)).unwrap_or_default();
    let table: toml::Table = toml::from_str(&text).unwrap_or_default();
    table
        .get("session")
        .and_then(|s| s.clone().try_into().ok())
        .unwrap_or_default()
}

fn autocompact(s: &Session) -> Result<Option<Value>, BosunError> {
    let (pct, window) = s.resolve(CONFIG_FILE)?;
    if pct.is_none() && window.is_none() {
        return Ok(None);
    }
    let mut cloud = Map::new();
    let mut exports = Vec::new();
    if let Some(w) = window {
        cloud.insert(
            "CLAUDE_CODE_AUTO_COMPACT_WINDOW".into(),
            json!(w.to_string()),
        );
        exports.push(format!("export CLAUDE_CODE_AUTO_COMPACT_WINDOW={w}"));
    }
    if let Some(p) = pct {
        exports.push(format!("export CLAUDE_AUTOCOMPACT_PCT_OVERRIDE={p}"));
    }
    let mut a = json!({"cloud_env": cloud, "local_exports": exports, "paste_into": ENV_PASTE});
    if pct.is_some() {
        a["note"] = json!(PCT_NOTE);
    }
    Ok(Some(a))
}
