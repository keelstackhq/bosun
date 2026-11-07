//! `bosun setup-snippet`: print the bootstrap to paste once as a cloud environment's setup
//! script (spec D23). It runs each checkout's own docs/setup-script.sh, so a release never
//! needs a new paste.

use serde_json::json;

use crate::output::{CmdResult, Outcome};

/// The bootstrap text; README.md shows it verbatim (a test checks).
pub const BOOTSTRAP: &str = include_str!("setup_bootstrap.sh");

/// One line the owner can send a client together with the script.
const CLIENT_MESSAGE: &str = "Paste this script once as the setup script of your Claude Code cloud environment (claude.ai/code > Environments > Setup script). You never change it again: each repo's own docs/setup-script.sh pins its tools.";

pub fn run() -> CmdResult {
    Ok(Outcome::data(json!({
        "script": BOOTSTRAP,
        "paste_into": "claude.ai/code > Environments > <environment> > Setup script",
        "repo_script": "docs/setup-script.sh",
        "client_message": CLIENT_MESSAGE,
    })))
}
