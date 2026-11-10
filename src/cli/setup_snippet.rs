//! `bosun setup-snippet`: print the bootstrap to paste once as a cloud environment's setup
//! script (spec D23). It runs each checkout's own docs/setup-script.sh, so a release never
//! needs a new paste. With `[session] autocompact_pct` it also prints the auto-compact variable.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::config::Config;
use crate::output::{BosunError, CmdResult, Outcome};

/// The bootstrap text; README.md shows it verbatim (a test checks).
pub const BOOTSTRAP: &str = include_str!("setup_bootstrap.sh");

/// One line the owner can send a client together with the script.
const CLIENT_MESSAGE: &str = "Paste this script once as the setup script of your Claude Code cloud environment (claude.ai/code > Environments > Setup script). You never change it again: each repo's own docs/setup-script.sh pins its tools.";

/// Where the auto-compact variable takes effect: the environment's variables, not the setup
/// script (its exports end with its shell) and not settings.json `env` (cloud sessions skip it).
const ENV_PASTE: &str = "claude.ai/code > Environments > <environment> > Environment variables (locally: export it in the shell that starts claude)";

const WINDOW_ALT: &str = "or set CLAUDE_CODE_AUTO_COMPACT_WINDOW=<100000-1000000 tokens> to shrink the window auto-compact measures against";

pub fn run(dir: Option<&Path>) -> CmdResult {
    let mut data = json!({
        "script": BOOTSTRAP,
        "paste_into": "claude.ai/code > Environments > <environment> > Setup script",
        "repo_script": "docs/setup-script.sh",
        "client_message": CLIENT_MESSAGE,
    });
    if let Some(pct) = autocompact_pct(dir)? {
        data["autocompact"] = autocompact(pct);
    }
    Ok(Outcome::data(data))
}

/// `[session] autocompact_pct` of the repo at `dir` (or the cwd); none without a config.
fn autocompact_pct(dir: Option<&Path>) -> Result<Option<u8>, BosunError> {
    let Some(start) = dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok())
    else {
        return Ok(None);
    };
    let root =
        crate::ctx::git_out(&start, &["rev-parse", "--show-toplevel"]).map_or(start, PathBuf::from);
    Ok(Config::load(&root)?.session.autocompact_pct)
}

fn autocompact(pct: u8) -> Value {
    json!({
        "export": format!("export CLAUDE_AUTOCOMPACT_PCT_OVERRIDE={pct}"),
        "env": {"CLAUDE_AUTOCOMPACT_PCT_OVERRIDE": pct.to_string()},
        "paste_into": ENV_PASTE,
        "alternative": WINDOW_ALT,
    })
}
