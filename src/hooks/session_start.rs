//! `bosun hook session-start`: join the registry, warm the lease cache and tell the session
//! where it stands (spec §4).

use std::path::Path;

use serde_json::json;

use super::cache;
use crate::app::{App, main_red};
use crate::registry::{self, JoinSpec};

fn output(text: &str) -> String {
    json!({"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": text}})
        .to_string()
}

pub fn run(cwd: Option<&Path>) -> String {
    let dir = cwd
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok());
    match context(dir.as_deref()) {
        Ok(text) => output(&text),
        Err(why) => {
            eprintln!("bosun: warning: session start: {why}");
            output(&format!(
                "bosun could not reach GitHub ({why}), so this session is not registered and \
                 edit locks may be stale. Set GH_TOKEN (or GITHUB_TOKEN) and BOSUN_REPO=owner/name, \
                 then run `bosun join`."
            ))
        }
    }
}

fn context(dir: Option<&Path>) -> Result<String, String> {
    let app = App::load(dir).map_err(|e| e.message)?;
    let (_, member) = registry::join(&app, JoinSpec::default()).map_err(|e| e.message)?;
    let file = dir.and_then(cache::path);
    let leases = cache::refresh(&app, file.as_deref()).map_err(|e| e.message)?;
    let mine: Vec<String> = leases
        .iter()
        .filter(|l| l.session == app.ctx.session)
        .map(|l| format!("#{}", l.issue))
        .collect();
    let own = if mine.is_empty() {
        "none".to_string()
    } else {
        mine.join(", ")
    };
    let halt = matches!(app.gh.variable("BOSUN_HALT"), Ok(Some(v)) if v.trim() == "1");
    let red = match main_red(app.gh.as_ref()).ok().flatten() {
        Some(n) => format!("Main is red: open main-red issue #{n}."),
        None => "No open main-red issue.".to_string(),
    };
    Ok(format!(
        "bosun: you are session {} (origin {}). Own live leases: {own}. BOSUN_HALT: {}. {red} \
         Run `bosun status` for the full picture.",
        member.session,
        member.origin,
        if halt { "1 (all work stopped)" } else { "off" },
    ))
}
