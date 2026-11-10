//! `bosun setup-snippet`: print the bootstrap to paste once as a cloud environment's setup
//! script (spec D23). It runs each checkout's own docs/setup-script.sh, so a release never
//! needs a new paste.

use std::path::PathBuf;

use serde_json::{Map, Value, json};

use super::SetupSnippetArgs;
use crate::output::{BosunError, CmdResult, Outcome};

/// The bootstrap text; README.md shows it verbatim (a test checks).
pub const BOOTSTRAP: &str = include_str!("setup_bootstrap.sh");

/// One line the owner can send a client together with the script.
const CLIENT_MESSAGE: &str = "Paste this script once as the setup script of your Claude Code cloud environment (claude.ai/code > Environments > Setup script). You never change it again: each repo's own docs/setup-script.sh pins its tools.";

/// The hooks bosun wants in every repo: (event, matcher, command). They equal the repo's own
/// `.claude/settings.json` entries (a test checks), so Claude Code runs each command once.
pub const HOOKS: [(&str, Option<&str>, &str); 4] = [
    ("SessionStart", None, "bosun hook session-start"),
    (
        "Stop",
        None,
        "out=$(bosun hook stop) && printf '%s\\n' \"$out\"; exit 0",
    ),
    (
        "PreToolUse",
        Some("Edit|Write|MultiEdit|NotebookEdit|Bash|mcp__github__.*"),
        "bosun hook pre-tool-use",
    ),
    (
        "PostToolUse",
        Some("AskUserQuestion"),
        "bosun hook post-tool-use",
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
    for (event, matcher, command) in HOOKS {
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
        g.insert(
            "hooks".into(),
            json!([{"type": "command", "command": command}]),
        );
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
        let tmp = path.with_extension("json.bosun-tmp");
        std::fs::write(&tmp, format!("{text}\n"))
            .and_then(|()| std::fs::rename(&tmp, &path))
            .map_err(|e| bad(e.to_string()))?;
    }
    Ok(Outcome::data(
        json!({"settings": shown.to_string(), "added": added}),
    ))
}

pub fn run(a: &SetupSnippetArgs) -> CmdResult {
    if a.install_hooks {
        return install_hooks(a.settings.clone());
    }
    Ok(Outcome::data(json!({
        "script": BOOTSTRAP,
        "paste_into": "claude.ai/code > Environments > <environment> > Setup script",
        "repo_script": "docs/setup-script.sh",
        "client_message": CLIENT_MESSAGE,
    })))
}
