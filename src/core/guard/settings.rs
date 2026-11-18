//! Settings-lock (spec §4): an agent may not edit the files that switch the guard off, the
//! repo's `.claude/settings*.json` and anything under the user's `~/.claude`, nor write
//! `disableAllHooks`. Best effort and lexical, like the rest of the guard; it fails closed.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use super::writes::command_of;
use super::{
    SEP, bash_writes, edited_path, prose, read_only, shell_words, split_heredocs, strings,
};

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

/// `path` without quotes and backslashes (`.cla''ude`), then cleaned lexically.
fn plain(path: &str) -> String {
    let bare: String = path
        .chars()
        .filter(|c| !matches!(c, '\'' | '"' | '\\'))
        .collect();
    normalize(&bare)
}

fn protected(path: &str) -> bool {
    PATH.is_match(&plain(path))
}

/// A word naming a target of a mutating command: a settings file, a glob that could reach one, or
/// the `.claude` directory itself (`rm -rf .claude`, `git checkout -- .claude`).
static TARGET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!("{}|(?:^|/)\\.claude/?$", TEXT.as_str())).expect("settings target regex")
});

/// Interpreters and shells: the code they run is only seen as text, so a target in it counts.
const INTERP: [&str; 12] = [
    "python", "python3", "node", "perl", "ruby", "php", "bash", "sh", "zsh", "dash", "eval", "deno",
];

/// MCP tools that write a repo file, by the name after the server prefix.
const FILE_TOOLS: [&str; 5] = [
    "create_or_update_file",
    "push_files",
    "delete_file",
    "update_file",
    "create_file",
];

fn target(word: &str) -> bool {
    TARGET.is_match(&plain(word))
}

/// The value of an option given as `-t X`, `-tX`, `--long X` or `--long=X`.
fn opt<'a>(args: &[&'a str], short: &str, long: &str) -> Option<&'a str> {
    args.iter().enumerate().find_map(|(i, a)| match *a {
        a if a == short || a == long => args.get(i + 1).copied(),
        a => a
            .strip_prefix(format!("{long}=").as_str())
            .or(a.strip_prefix(short)),
    })
}

/// The settings-lock for one tool call; `None` allows it.
pub(super) fn settings_rule(tool: &str, input: &Value) -> Option<String> {
    if tool == "Bash" {
        let command = input.get("command").and_then(Value::as_str).unwrap_or("");
        return bash(command);
    }
    let name = tool.rsplit("__").next().unwrap_or(tool);
    let hit = if FILE_TOOLS.contains(&name) {
        let files = input.get("files").and_then(Value::as_array);
        let paths = files.into_iter().flatten().chain([input]);
        paths.filter_map(|f| f.get("path")?.as_str()).any(protected)
    } else {
        let path = edited_path(tool, input)?;
        // A `disableAllHooks` key only counts in a JSON file: a spec or test may name it.
        let sets = path.ends_with(".json") && strings(input).iter().any(|t| HOOKS_OFF.is_match(t));
        protected(&path) || sets
    };
    hit.then(|| format!("`{tool}` edits the guard's own settings: {FIX}"))
}

/// Whether one command word, with its arguments, mutates a target (never a read of one).
fn mutates(cmd: &str, args: &[&str]) -> bool {
    let any = |a: &[&str]| a.iter().any(|w| !w.starts_with('>') && target(w));
    match cmd {
        "rm" | "rmdir" | "unlink" | "shred" | "truncate" | "touch" | "chmod" | "chown" | "ln"
        | "mv" => any(args),
        // `bash_writes` skips `~` and `$HOME` targets: judge them here (a path word only).
        "cp" | "install" => {
            let last = args.iter().rev().find(|a| !a.starts_with('-'));
            opt(args, "-t", "--target-directory").is_some_and(target)
                || last.is_some_and(|a| protected(a))
        }
        "tee" => args.iter().any(|a| protected(a)),
        "sed"
            if args.iter().any(|a| {
                a.starts_with("--in-place")
                    || (a.starts_with('-') && !a.starts_with("--") && a.contains('i'))
            }) =>
        {
            args.iter().any(|a| protected(a))
        }
        "find" => {
            let acts = ["-delete", "-exec", "-execdir", "-ok", "-fprint", "-fprintf"];
            let named = args
                .iter()
                .any(|a| a.starts_with("settings") && a.ends_with(".json"));
            args.iter().any(|a| acts.contains(a)) && (any(args) || named)
        }
        "tar" => {
            let x = |a: &&str| {
                matches!(*a, "--extract" | "--get")
                    || (a.starts_with('-') && !a.starts_with("--") && a.contains('x'))
            };
            let bundled = args
                .first()
                .is_some_and(|a| !a.starts_with('-') && a.contains('x'));
            (args.iter().any(x) || bundled)
                && (any(args) || opt(args, "-C", "--directory").is_some_and(target))
        }
        "git" => {
            let opt = |i: usize| i > 0 && matches!(args[i - 1], "-C" | "-c");
            let sub = (0..args.len()).find(|&i| !args[i].starts_with('-') && !opt(i));
            let ops = ["checkout", "restore", "stash", "reset", "clean", "rm", "mv"];
            sub.is_some_and(|i| ops.contains(&args[i]) && any(&args[i + 1..]))
        }
        _ => false,
    }
}

/// Whether a redirection in the segment (`>f`, `>> f`, `2>f`) writes a target; unlike
/// `bash_writes` it also sees `~` and `$HOME` paths.
fn redirects(seg: &[String]) -> bool {
    seg.iter().enumerate().any(|(i, w)| {
        let op = w.trim_start_matches(|c: char| c.is_ascii_digit() || c == '&');
        let rest = op.strip_prefix('>').map(|r| r.trim_start_matches('>'));
        let to = rest.map(|r| {
            if r.is_empty() {
                seg.get(i + 1).map_or("", |n| n)
            } else {
                r
            }
        });
        to.is_some_and(target)
    })
}

fn bash(command: &str) -> Option<String> {
    let text = prose::strip_prose(command);
    let dequoted: String = text
        .chars()
        .filter(|c| !matches!(c, '\'' | '"' | '\\'))
        .collect();
    let words = shell_words(&split_heredocs(&text).0);
    let (mut interp, mut mutated) = (false, false);
    for seg in words.split(|w| w == SEP) {
        let (cmd, args, _) = command_of(seg);
        interp |= INTERP.contains(&cmd);
        mutated |= mutates(cmd, &args) || redirects(seg);
    }
    // `read_only` ignores redirections: a `>` in code that runs counts as a write.
    let reads = read_only(&text, 0) && !text.contains('>');
    let sets = (HOOKS_OFF.is_match(&text) || HOOKS_OFF.is_match(&dequoted))
        && dequoted.contains(".json")
        && !reads;
    let code = interp && TEXT.is_match(&dequoted) && !reads;
    let writes = bash_writes(command).iter().any(|p| protected(p));
    (mutated || code || sets || writes)
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
        assert!(sh("printf 'disableAllHooks=1' | tee out.json").is_some());
        // Prose in a spec or a test, or a non-JSON file, may name the key.
        let spec =
            json!({"file_path": "docs/spec/04.md", "new_string": "\"disableAllHooks\": true"});
        assert!(settings_rule("Edit", &spec).is_none());
        assert!(sh("sed -i 's/a/disableAllHooks=1/' tests/x.rs").is_none());
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

    #[test]
    fn destructive_and_indirect_commands_are_denied() {
        for c in [
            "rm -rf .claude",
            "mv .claude/settings.json /tmp/x",
            "mv x .claude/settings.local.json",
            "find .claude -delete",
            "find . -name 'settings*.json' -delete",
            "find ~/.claude -exec rm {} +",
            "git checkout -- .claude",
            "git checkout HEAD~1 -- .claude/settings.json",
            "git -C . restore .claude/settings.json",
            "git stash push -- .claude/settings.json",
            "git reset HEAD~1 -- .claude",
            "git rm .claude/settings.json",
            "cp -t .claude x.json",
            "cp --target-directory=.claude/ x.json",
            "install -t ~/.claude x",
            "tar -xf a.tar -C .claude",
            "tar xf a.tar -C.claude",
            "ln -s x .claude/settings.json",
            "ln -sfn /tmp/d .claude",
            "ln -s .claude/settings.json link",
        ] {
            assert!(sh(c).is_some(), "{c}");
        }
    }

    #[test]
    fn reads_and_prose_are_not_over_denied() {
        for c in [
            "ls .claude 2>/dev/null",
            "cat .claude/settings.json 2>/dev/null",
            "grep hooks .claude/settings.json > /tmp/out.txt",
            "cp .claude/settings.json /tmp/x",
            "cp -r .claude/commands /tmp/y",
            "tar -cf a.tar -C .claude .",
            "sed -i 's/a/b/' docs/spec/04-lifecycle.md",
            "sed -i 's#x#.claude/settings.json#' docs/spec/04-lifecycle.md",
            "echo '.claude/settings.json' > notes.txt",
            "git checkout -- src",
            "git add .claude/settings.json",
            "find . -name '*.rs' -delete",
            "rm -rf target",
        ] {
            assert!(sh(c).is_none(), "{c}");
        }
        let edit = json!({"file_path": "tests/x.rs", "new_string": ".claude/settings.json"});
        assert!(settings_rule("Edit", &edit).is_none());
    }

    #[test]
    fn github_file_tools_are_denied_on_settings() {
        let one = |p: &str| json!({"owner": "o", "repo": "r", "path": p, "content": "{}"});
        for t in [
            "mcp__github__create_or_update_file",
            "mcp__github__delete_file",
        ] {
            assert!(
                settings_rule(t, &one(".claude/settings.json")).is_some(),
                "{t}"
            );
            assert!(settings_rule(t, &one("a/.claude/settings.local.json")).is_some());
            assert!(settings_rule(t, &one("docs/spec.md")).is_none());
        }
        let push = |p: &str| json!({"files": [{"path": "a.md"}, {"path": p, "content": "{}"}]});
        let t = "mcp__github__push_files";
        assert!(settings_rule(t, &push(".claude/settings.json")).is_some());
        assert!(settings_rule(t, &push("src/x.rs")).is_none());
    }
}
