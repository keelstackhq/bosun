//! `bosun hook session-start`: join the registry, warm the lease cache and tell the session
//! where it stands (spec §4).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::cache;
use super::session_rules::rules_context;
use crate::app::{App, halt_reason, main_red};
use crate::cli::lesson;
use crate::core::lessons;
use crate::registry::{self, JoinSpec};

fn output(text: &str) -> String {
    json!({"hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": text}})
        .to_string()
}

/// True when the repo at `cwd` has no `.bosun/version`: bosun was never set up here.
pub fn unadopted(cwd: &Path) -> bool {
    let root = crate::ctx::git_out(cwd, &["rev-parse", "--show-toplevel"])
        .map_or_else(|| cwd.to_path_buf(), PathBuf::from);
    !root.join(".bosun/version").exists()
}

const NOT_SET_UP: &str = "bosun is not set up here: run `bosun adopt --policy autonomous \
    --owner <login>` (the owner's GitHub login) and open the setup PR. Until then bosun has no \
    registry, leases or approvals in this repo.";

pub fn run(cwd: Option<&Path>, input: &Value) -> String {
    let dir = cwd
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok());
    if dir.as_deref().is_some_and(unadopted) {
        return output(NOT_SET_UP);
    }
    if let Some(d) = dir.as_deref()
        && let Err(e) = crate::cli::trailers::install(d)
    {
        eprintln!("bosun: trailers hook: {}", e.message);
    }
    let lessons = format!(
        "{}{}{}",
        skew_block(dir.as_deref()),
        lessons_block(dir.as_deref()),
        pin_block(dir.as_deref())
    );
    let registered = context(dir.as_deref());
    // After compaction the summary may have lost the state: add it back (spec §4). Built after
    // `context`, which refreshed the lease cache the block reads.
    let lessons = match dir.as_deref() {
        Some(d) if input.get("source").and_then(Value::as_str) == Some("compact") => {
            let block = super::compact::safe_block(d);
            format!(
                "{lessons}{}",
                block.map(|b| format!("\n{b}")).unwrap_or_default()
            )
        }
        _ => lessons,
    };
    match registered {
        Ok(text) => {
            // The janitor: expired leases go back to ready (spec §4); a failure is only a warning.
            if let Err(e) = crate::cli::reap::run(dir.as_deref()) {
                eprintln!("bosun: warning: reap: {}", e.message);
            }
            output(&format!(
                "{text}{lessons}\n{}",
                rules_context(dir.as_deref())
            ))
        }
        Err(why) if why.contains(".bosun/") => {
            eprintln!("bosun: warning: session start: {why}");
            output(&format!(
                "bosun cannot read its config, so this session is not registered: {why}. Fix the \
                 file, then run `bosun join`.{lessons}\n{}",
                rules_context(dir.as_deref())
            ))
        }
        Err(why) => {
            eprintln!("bosun: warning: session start: {why}");
            output(&format!(
                "bosun could not reach GitHub ({why}), so this session is not registered and \
                 edit locks may be stale. Set GH_TOKEN (or GITHUB_TOKEN) and BOSUN_REPO=owner/name, \
                 then run `bosun join`.{lessons}\n{}",
                rules_context(dir.as_deref())
            ))
        }
    }
}

/// The repo's global lessons (at most 8) as a block to append; empty when there are none or
/// `docs/lessons.toml` is missing or unreadable. Read before any GitHub call.
fn lessons_block(dir: Option<&Path>) -> String {
    let root = dir.map(|d| {
        crate::ctx::git_out(d, &["rev-parse", "--show-toplevel"])
            .map_or_else(|| d.to_path_buf(), PathBuf::from)
    });
    let Some(store) = root.and_then(|d| lesson::load(&d).ok()) else {
        return String::new();
    };
    let lines: Vec<String> = lessons::globals(&store)
        .into_iter()
        .take(lessons::MAX_GLOBAL)
        .map(lessons::line)
        .collect();
    if lines.is_empty() {
        String::new()
    } else {
        format!("\nLessons:\n{}", lines.join("\n"))
    }
}

/// One line when the installed binary differs from the repo's `.bosun/version` pin.
fn skew_block(dir: Option<&Path>) -> String {
    let root = dir.map(|d| {
        crate::ctx::git_out(d, &["rev-parse", "--show-toplevel"])
            .map_or_else(|| d.to_path_buf(), PathBuf::from)
    });
    let pinned = root.and_then(|r| std::fs::read_to_string(r.join(".bosun/version")).ok());
    match pinned.as_deref().map(str::trim) {
        Some(p) if !p.is_empty() && p != env!("CARGO_PKG_VERSION") => format!(
            "\ninstalled {}, repo pins {p}: run bash docs/setup-script.sh",
            env!("CARGO_PKG_VERSION")
        ),
        _ => String::new(),
    }
}

/// One line when a tool pin is behind npm (spec D35); empty otherwise, any error goes to stderr.
fn pin_block(dir: Option<&Path>) -> String {
    let Some(dir) = dir else {
        return String::new();
    };
    // BOSUN_PIN_CHECK=0 opts out of the lookup, so it must not read an old answer either.
    let store = (std::env::var("BOSUN_PIN_CHECK").as_deref() != Ok("0"))
        .then(|| cache::path(dir))
        .flatten()
        .map(|p| p.with_file_name("pin-notice.json"));
    let now = crate::time::Timestamp::now().0;
    if let Some(hit) = store.as_deref().and_then(|p| read_notice(p, now)) {
        return hit.map(|l| format!("\n{l}")).unwrap_or_default();
    }
    match crate::cli::pin::session_notice(dir) {
        Ok(line) => {
            if let Some(p) = store.as_deref() {
                let body = json!({"at": now, "line": line});
                let _ = std::fs::create_dir_all(p.parent().unwrap_or(dir))
                    .and_then(|()| std::fs::write(p, body.to_string()));
            }
            line.map(|l| format!("\n{l}")).unwrap_or_default()
        }
        Err(e) => {
            eprintln!("bosun: warning: pin check: {}", e.message);
            String::new()
        }
    }
}

/// npm is asked at most once a day per checkout: the answer is kept next to the lease cache.
const PIN_TTL_SECS: i64 = 86_400;

/// The kept answer (`Some(None)` is "nothing behind"); `None` when missing, old or unreadable.
fn read_notice(file: &Path, now: i64) -> Option<Option<String>> {
    let v: Value = serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()?;
    let at = v.get("at")?.as_i64()?;
    (0..PIN_TTL_SECS)
        .contains(&(now - at))
        .then(|| v.get("line").and_then(Value::as_str).map(String::from))
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
    let halt = !matches!(halt_reason(app.gh.as_ref()), Ok(None));
    let red = match main_red(app.gh.as_ref()).ok().flatten() {
        Some(n) => format!("Main is red: open main-red issue #{n}."),
        None => "No open main-red issue.".to_string(),
    };
    Ok(format!(
        "bosun: you are session {} (origin {}). Own live leases: {own}. Kill switch: {}. {red} \
         Run `bosun status` for the full picture.",
        member.session,
        member.origin,
        if halt {
            "on or unreadable (work commands exit 4)"
        } else {
            "off"
        },
    ))
}
