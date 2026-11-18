//! Settings-lock (spec §4): an agent may not edit the files that switch the guard off, the
//! repo's `.claude/settings*.json` and anything under the user's `~/.claude`, nor write
//! `disableAllHooks`. Best effort and lexical, like the rest of the guard; it fails closed.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::{bash_writes, edited_path, prose, read_only, strings};

const FIX: &str = "the hooks and permissions in .claude/settings*.json and ~/.claude are the \
    owner's: ask the orchestrator to ask the owner in chat to change them, and do not set \
    disableAllHooks";

const HOME: &str = r"(?:~|\$\{?HOME\}?|/root|/home/[^/\s]+|/Users/[^/\s]+)";

/// A path (file tool or redirect target) the lock covers.
static PATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^(?:{HOME}/\.claude(?:/|$)|(?:.*/)?\.claude/settings[^/]*\.json$)"
    ))
    .expect("settings path regex")
});

/// The same targets anywhere in a command's text, with globs that could reach them.
static TEXT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"{HOME}/\.claude\b|\.claude/(?:set|s\*|\*|\?|\[)|\.claude/settings[^/\s]*\.json"
    ))
    .expect("settings text regex")
});

/// A `disableAllHooks` key or assignment (`"disableAllHooks": true`, `.disableAllHooks=true`).
static HOOKS_OFF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)disableAllHooks["'\\]{0,3}\s*[:=]"#).expect("disableAllHooks regex")
});

/// Lexical clean-up: `//`, `/./` and `x/../` collapse; the result is only used for matching.
fn normalize(path: &str) -> String {
    let abs = path.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if out.last().is_some_and(|l| *l != "..") => {
                out.pop();
            }
            p => out.push(p),
        }
    }
    format!("{}{}", if abs { "/" } else { "" }, out.join("/"))
}

fn protected(path: &str) -> bool {
    let plain: String = path
        .chars()
        .filter(|c| !matches!(c, '\'' | '"' | '\\'))
        .collect();
    PATH.is_match(&normalize(&plain))
}

/// The settings-lock for one tool call; `None` allows it.
pub(super) fn settings_rule(tool: &str, input: &Value) -> Option<String> {
    if tool == "Bash" {
        let command = input.get("command").and_then(Value::as_str).unwrap_or("");
        return bash(command);
    }
    let path = edited_path(tool, input)?;
    let sets = strings(input).iter().any(|t| HOOKS_OFF.is_match(t));
    (protected(&path) || sets).then(|| format!("`{tool}` edits the guard's own settings: {FIX}"))
}

fn bash(command: &str) -> Option<String> {
    let text = prose::strip_prose(command);
    let dequoted: String = text
        .chars()
        .filter(|c| !matches!(c, '\'' | '"' | '\\'))
        .collect();
    let hit =
        TEXT.is_match(&dequoted) || HOOKS_OFF.is_match(&text) || HOOKS_OFF.is_match(&dequoted);
    let writes = bash_writes(command).iter().any(|p| protected(p));
    // `read_only` ignores redirections, so any `>` counts as a write when a target is named.
    ((hit && (text.contains('>') || !read_only(&text, 0))) || writes)
        .then(|| format!("the command writes the guard's own settings: {FIX}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn file(tool: &str, path: &str) -> Option<String> {
        settings_rule(tool, &json!({"file_path": path, "content": "{}"}))
    }
    fn sh(cmd: &str) -> Option<String> {
        settings_rule("Bash", &json!({"command": cmd}))
    }

    #[test]
    fn file_tools_are_denied_on_settings_and_home_claude() {
        for tool in ["Edit", "Write", "MultiEdit"] {
            for p in [
                ".claude/settings.json",
                ".claude/settings.local.json",
                "/repo/.claude/settings.json",
                "sub/../.claude//settings.json",
                "/repo/./.claude/settings.x.json",
                "~/.claude/settings.json",
                "$HOME/.claude/hooks/x.sh",
                "${HOME}/.claude/CLAUDE.md",
                "/root/.claude/settings.json",
                "/home/dev/.claude/commands/a.md",
                "/Users/dev/.claude",
            ] {
                assert!(file(tool, p).is_some(), "{tool} {p}");
            }
        }
        let why = file("Edit", ".claude/settings.json").unwrap();
        assert!(why.contains("ask the orchestrator"), "{why}");
    }

    #[test]
    fn other_paths_and_reads_pass() {
        for p in [
            ".claude/commands/x.md",
            "src/settings.json",
            ".claude/settings.md",
            "/repo/.claude",
        ] {
            assert!(file("Edit", p).is_none(), "{p}");
        }
        for c in [
            "cat .claude/settings.json",
            "grep hooks ~/.claude/settings.json",
            "git commit -m 'note .claude/settings.json'",
        ] {
            assert!(sh(c).is_none(), "{c}");
        }
    }

    #[test]
    fn disable_all_hooks_is_denied_in_any_file_or_command() {
        for t in ["Edit", "Write", "MultiEdit"] {
            let input = json!({"file_path": "x.json", "new_string": "{\"disableAllHooks\": true}"});
            assert!(settings_rule(t, &input).is_some());
        }
        let prose = json!({"file_path": "docs/x.md", "new_string": "the disableAllHooks key"});
        assert!(settings_rule("Write", &prose).is_none());
        assert!(sh("echo '{\"disableAllHooks\":true}' > x.json").is_some());
        assert!(sh("jq '.disableAllHooks = true' a.json > b.json").is_some());
        assert!(sh("printf 'disableAllHooks=1' | tee out").is_some());
    }

    #[test]
    fn bash_writes_are_denied_dequoted() {
        for c in [
            "echo {} > .claude/settings.json",
            "echo {} >> ~/.claude/settings.json",
            "sed -i s/a/b/ .claude/settings.local.json",
            "sed -i '' 's/a/b/' $HOME/.claude/settings.json",
            "echo x | tee ~/.claude/settings.json",
            "cp x .claude/settings.json",
            "mv x \"$HOME/.claude/settings.json\"",
            "rm .claude/settings.json",
            "rm -rf ~/.claude",
            "rm -rf $HOME/.claude/hooks",
            "rm .cla''ude/settings.json",
            "rm .claude/set\\tings.json",
            "cd x && rm /root/.claude/settings.json",
            "bash -c 'echo {} > .claude/settings.json'",
            "rm .claude/sett*",
            "rm .claude/*",
            "python3 -c \"open('.claude/settings.json','w').write('{}')\"",
        ] {
            assert!(sh(c).is_some(), "{c}");
        }
    }
}
