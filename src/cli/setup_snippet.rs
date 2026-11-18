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

fn has_command(groups: &[Value], command: &str) -> bool {
    groups.iter().any(|g| {
        g.get("hooks").and_then(Value::as_array).is_some_and(|hs| {
            hs.iter()
                .any(|h| h.get("command").and_then(Value::as_str) == Some(command))
        })
    })
}

/// Add each missing hook to `settings`, matched by exact command string; other keys and
/// entries stay. Returns the events it added to.
fn merge_hooks(settings: &mut Value) -> Result<Vec<&'static str>, String> {
    let root = settings
        .as_object_mut()
        .ok_or("the settings are not a JSON object")?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or("\"hooks\" is not an object")?;
    let mut added = Vec::new();
    for (event, matcher, command, timeout) in HOOKS {
        let groups = hooks
            .entry(event)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or_else(|| format!("hooks.{event} is not an array"))?;
        if has_command(groups, command) {
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
    Ok(added)
}

fn install_hooks(settings: Option<PathBuf>) -> CmdResult {
    let path = settings
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".claude/settings.json"))
        })
        .ok_or_else(|| BosunError::error("HOME is not set", "pass --settings <path>"))?;
    // Edit a symlinked settings file in place: the link stays and the target is rewritten.
    let path = std::fs::canonicalize(&path).unwrap_or(path);
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
    let added = merge_hooks(&mut doc).map_err(bad)?;
    if !added.is_empty() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| bad(e.to_string()))?;
        }
        let text = serde_json::to_string_pretty(&doc).map_err(|e| bad(e.to_string()))?;
        // Write beside, then rename: a crash never leaves half a settings file.
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".bosun-tmp-{}", std::process::id()));
        let tmp = path.with_file_name(name);
        std::fs::write(&tmp, format!("{text}\n"))
            .and_then(|()| match std::fs::metadata(&path) {
                Ok(m) => std::fs::set_permissions(&tmp, m.permissions()),
                Err(_) => Ok(()),
            })
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                bad(e.to_string())
            })?;
    }
    Ok(Outcome::data(
        json!({"settings": shown.to_string(), "added": added}),
    ))
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
