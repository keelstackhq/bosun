//! `bosun setup-snippet`: print the bootstrap to paste once as a cloud environment's setup
//! script (spec D23). It runs each checkout's own docs/setup-script.sh, so a release never
//! needs a new paste. With `[session]` auto-compact keys it also prints their variables.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::SetupSnippetArgs;
use crate::config::{CONFIG_FILE, Session};
use crate::output::{BosunError, CmdResult, Outcome};

/// The bootstrap text; README.md shows it verbatim (a test checks).
pub const BOOTSTRAP: &str = include_str!("setup_bootstrap.sh");

/// The short form: each checkout's own docs/setup-script.sh (it installs the pinned bosun and
/// checks every sha512), never failing the session. No fallback for a repo without that script.
pub const SHORT: &str = r#"for d in . ./*/; do
  [ -f "$d/.bosun/version" ] && [ -f "$d/docs/setup-script.sh" ] || continue
  (cd "$d" && timeout -k 10 1800 bash docs/setup-script.sh </dev/null) || echo "bosun: $d/docs/setup-script.sh failed" >&2
done; exit 0
"#;

/// One line the owner can send a client together with the script.
const CLIENT_MESSAGE: &str = "Paste this script once as the setup script of your Claude Code cloud environment (claude.ai/code > Environments > Setup script). You never change it again: each repo's own docs/setup-script.sh pins its tools.";

/// Where `cloud_env` goes: the environment's variables, not the setup script (its exports end
/// with its shell) and not settings.json `env` (cloud sessions skip it).
const ENV_PASTE: &str = "claude.ai/code > Environments > <environment> > Environment variables";

/// Cloud sessions set CLAUDE_AUTOCOMPACT_PCT_OVERRIDE themselves (Claude Code cloud docs).
const PCT_NOTE: &str = "autocompact_pct is for local sessions only: a cloud session sets CLAUDE_AUTOCOMPACT_PCT_OVERRIDE itself and ignores an environment value; use autocompact_window for cloud";

/// The hooks bosun wants in every repo: (event, matcher, command). They equal the repo's own
/// `.claude/settings.json` entries (a test checks), so Claude Code runs each command once.
pub const HOOKS: [(&str, Option<&str>, &str, Option<u32>); 5] = [
    ("SessionStart", None, "bosun hook session-start", None),
    (
        "Stop",
        None,
        "out=$(bosun hook stop) && printf '%s\\n' \"$out\"; exit 0",
        None,
    ),
    (
        "PreCompact",
        None,
        "out=$(bosun hook pre-compact) && printf '%s\\n' \"$out\"; exit 0",
        Some(15),
    ),
    (
        "PreToolUse",
        Some("Edit|Write|MultiEdit|NotebookEdit|Bash|mcp__github__.*"),
        "bosun hook pre-tool-use",
        None,
    ),
    (
        "PostToolUse",
        Some("AskUserQuestion"),
        "bosun hook post-tool-use",
        None,
    ),
];

/// The events whose bosun hook command is not in `settings` (any matcher).
pub fn missing_events(settings: &Value) -> Vec<&'static str> {
    HOOKS
        .iter()
        .filter(|(event, _, command, _)| {
            !settings["hooks"][event].as_array().is_some_and(|gs| {
                gs.iter().any(|g| {
                    g["hooks"]
                        .as_array()
                        .is_some_and(|hs| hs.iter().any(|h| h["command"].as_str() == Some(command)))
                })
            })
        })
        .map(|(event, ..)| *event)
        .collect()
}

/// Add each missing hook to `settings`, matched by exact command string; an entry with the
/// command but another matcher or timeout is updated in place (never duplicated); other keys and
/// entries stay. Returns the events added to and the events updated.
fn merge_hooks(settings: &mut Value) -> Result<(Vec<&'static str>, Vec<&'static str>), String> {
    let root = settings
        .as_object_mut()
        .ok_or("the settings are not a JSON object")?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or("\"hooks\" is not an object")?;
    let (mut added, mut updated) = (Vec::new(), Vec::new());
    for (event, matcher, command, timeout) in HOOKS {
        let groups = hooks
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| format!("hooks.{event} is not an array"))?;
        let found = groups.iter_mut().find_map(|g| {
            let at = g
                .get("hooks")?
                .as_array()?
                .iter()
                .position(|h| h.get("command").and_then(Value::as_str) == Some(command))?;
            Some((g, at))
        });
        if let Some((g, at)) = found {
            let same = g.get("matcher").and_then(Value::as_str) == matcher;
            let mut changed = set_key(g, "matcher", same, matcher.map(|m| json!(m)));
            let h = &mut g["hooks"][at];
            let same = h.get("timeout").and_then(Value::as_u64) == timeout.map(u64::from);
            changed |= set_key(h, "timeout", same, timeout.map(|t| json!(t)));
            if changed {
                updated.push(event);
            }
            continue;
        }
        let mut g = Map::new();
        if let Some(m) = matcher {
            g.insert("matcher".into(), json!(m));
        }
        let mut h = json!({"type": "command", "command": command});
        if let Some(t) = timeout {
            h["timeout"] = json!(t);
        }
        g.insert("hooks".into(), json!([h]));
        groups.push(Value::Object(g));
        added.push(event);
    }
    Ok((added, updated))
}

/// Unless `same`, set `o[key]` to `want` (remove it for `None`); whether it changed.
fn set_key(o: &mut Value, key: &str, same: bool, want: Option<Value>) -> bool {
    match want {
        _ if same => {}
        Some(v) => o[key] = v,
        None => drop(o.as_object_mut().and_then(|m| m.remove(key))),
    }
    !same
}

fn install_hooks(settings: Option<PathBuf>) -> CmdResult {
    let path = settings
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude/settings.json"))
        })
        .ok_or_else(|| BosunError::error("HOME is not set", "pass --settings <path>"))?;
    // A symlinked settings file is written through: the link stays and the target is rewritten.
    // A dangling link is an error and is left untouched.
    let is_link = std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink());
    let path = match std::fs::canonicalize(&path) {
        Ok(p) => p,
        Err(_) if is_link => {
            return Err(BosunError::error(
                format!(
                    "{}: a symlink to a file that does not exist",
                    path.display()
                ),
                "point the link at a real file or remove it, then rerun `bosun setup-snippet --install-hooks`",
            ));
        }
        Err(_) => path,
    };
    let shown = path.display();
    let bad = |why: String| {
        BosunError::error(
            format!("{shown}: {why}"),
            format!("fix or move {shown}, then rerun `bosun setup-snippet --install-hooks`"),
        )
    };
    let mut doc = match std::fs::read_to_string(&path) {
        Ok(t) if t.trim().is_empty() => json!({}),
        Ok(t) => serde_json::from_str(&t).map_err(|e| bad(format!("not valid JSON ({e})")))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(bad(e.to_string())),
    };
    let (added, updated) = merge_hooks(&mut doc).map_err(bad)?;
    if !added.is_empty() || !updated.is_empty() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| bad(e.to_string()))?;
        }
        let text = serde_json::to_string_pretty(&doc).map_err(|e| bad(e.to_string()))?;
        // Write beside, then rename: a crash never leaves half a settings file. The temp file is
        // created private (0600) before any content is written, then takes the target's mode.
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".bosun-tmp-{}", std::process::id()));
        let tmp = path.with_file_name(name);
        let write = || -> std::io::Result<()> {
            use std::io::Write;
            let mut f = private_file(&tmp)?;
            f.write_all(format!("{text}\n").as_bytes())?;
            if let Ok(m) = std::fs::metadata(&path) {
                std::fs::set_permissions(&tmp, m.permissions())?;
            }
            std::fs::rename(&tmp, &path)
        };
        write().map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            bad(e.to_string())
        })?;
    }
    Ok(Outcome::data(
        json!({"settings": shown.to_string(), "added": added, "updated": updated}),
    ))
}

/// A new file open for writing, mode 0600 from the start on unix.
fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

pub fn run(dir: Option<&Path>, a: &SetupSnippetArgs) -> CmdResult {
    if a.install_hooks {
        return install_hooks(a.settings.clone());
    }
    let mut data = json!({
        "script": BOOTSTRAP,
        "short": SHORT,
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
