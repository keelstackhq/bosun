//! Sticky refusals (spec §4): a bosun refusal a script swallowed is shown again on the next
//! tool call. Pure: which exits stick, what clears them, and which tool calls write.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One refusal of one session's command, as the marker file holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refusal {
    pub session: String,
    pub command: String,
    pub exit_code: u8,
    pub fix: String,
    pub at: String,
    /// The hook already injected it once.
    #[serde(default)]
    pub shown: bool,
    /// A kill-switch halt: writes are refused while it stands.
    #[serde(default)]
    pub lock: bool,
}

pub const HALTED: u8 = 4;

/// Exits that stick: halted, needs-human, limit; conflict only on claim.
pub fn sticky(command: &str, exit_code: u8) -> bool {
    matches!(exit_code, 4..=6) || (exit_code == 3 && command == "claim")
}

/// The fixed phrases of a kill-switch halt (`app::halt_reason`), in its message or fix.
const KILL_SWITCH: &[&str] = &[
    "halt issue #",
    "bosun:halt issues",
    "BOSUN_HALT",
    "stays halted",
];

/// Whether a refusal locks writes: only a kill-switch halt. Main-red and process-red halts
/// leave writes open for the fix they ask for.
pub fn locks(exit_code: u8, message: &str, fix: &str) -> bool {
    exit_code == HALTED
        && KILL_SWITCH
            .iter()
            .any(|p| message.contains(p) || fix.contains(p))
}

/// Whether `session`'s marker belongs to the hook's `caller`: the same id, or a task session
/// `<caller>/<task>` (`BOSUN_SESSION` set per command never reaches the hook).
pub fn mine(session: &str, caller: &str) -> bool {
    session == caller
        || session
            .strip_prefix(caller)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Whether a successful `command` (its `data`) clears `r`: the same command, a command the fix
/// names as `bosun <command>`, or for a halt `bosun status` showing no halt and no main-red.
pub fn clears(r: &Refusal, command: &str, data: &Value) -> bool {
    if command == r.command || names(&r.fix, command) {
        return true;
    }
    let none = |k: &str| data.get(k).is_none_or(|v| v.is_null() || v == false);
    command == "status"
        && r.exit_code == HALTED
        && data.get("halted") == Some(&Value::Bool(false))
        && none("main_red")
        && none("process_red")
}

fn names(fix: &str, command: &str) -> bool {
    let words: Vec<&str> = fix
        .split(|c: char| c.is_whitespace() || c == '`')
        .filter(|w| !w.is_empty())
        .collect();
    let cmd: Vec<&str> = command.split_whitespace().collect();
    words
        .windows(cmd.len() + 1)
        .any(|w| w[0] == "bosun" && w[1..] == cmd[..])
}

/// The context line the hook injects for one refusal.
pub fn context(r: &Refusal) -> String {
    let halt = if r.lock {
        "; writes and pushes are refused until `bosun status` shows the halt lifted"
    } else {
        ""
    };
    format!(
        "bosun refusal: `bosun {}` exited {} at {} (a script may have hidden it). Fix: {}{halt}",
        r.command,
        exit_word(r.exit_code),
        r.at,
        r.fix
    )
}

fn exit_word(code: u8) -> String {
    let name = match code {
        3 => "conflict",
        4 => "halted",
        5 => "needs-human",
        6 => "limit",
        _ => "error",
    };
    format!("exit {code} ({name})")
}

/// The deny reason for a write while halted.
pub fn halted_deny(r: &Refusal) -> String {
    format!(
        "bosun is halted (`bosun {}` exited 4): {}; run `bosun status` once the halt is lifted",
        r.command, r.fix
    )
}

/// Whether a tool call writes or pushes: file edits, GitHub MCP writes, and Bash `git` or
/// `gh` subcommands that change a repo, a PR or an issue.
pub fn is_write(tool: &str, input: &Value) -> bool {
    if matches!(tool, "Edit" | "Write" | "MultiEdit" | "NotebookEdit") {
        return true;
    }
    if let Some(name) = tool.strip_prefix("mcp__github__") {
        let read = ["get_", "list_", "search_"]
            .iter()
            .any(|p| name.starts_with(p))
            || name.ends_with("_read");
        return !read;
    }
    if tool != "Bash" {
        return false;
    }
    let command = input.get("command").and_then(Value::as_str).unwrap_or("");
    let words: Vec<&str> = command
        .split(|c: char| c.is_whitespace() || ";&|()`$\"'".contains(c))
        .filter(|w| !w.is_empty())
        .collect();
    const GIT: &[&str] = &[
        "push",
        "commit",
        "tag",
        "merge",
        "rebase",
        "cherry-pick",
        "am",
    ];
    const GH: &[(&str, &[&str])] = &[
        (
            "pr",
            &["create", "merge", "edit", "ready", "close", "comment"],
        ),
        ("issue", &["create", "edit", "close", "comment"]),
        ("release", &["create", "upload"]),
    ];
    words.iter().enumerate().any(|(i, w)| {
        let rest = || git_sub(&words[i + 1..]);
        match w.rsplit('/').next().unwrap_or(w) {
            "git" => rest().is_some_and(|s| GIT.contains(&s)),
            "gh" => words.get(i + 1).is_some_and(|area| {
                GH.iter().any(|(a, subs)| {
                    a == area && words.get(i + 2).is_some_and(|s| subs.contains(s))
                })
            }),
            _ => false,
        }
    })
}

/// The git subcommand after global options (`-C dir`, `-c k=v`, `--flag`).
fn git_sub<'a>(rest: &[&'a str]) -> Option<&'a str> {
    let mut it = rest.iter();
    while let Some(w) = it.next() {
        match *w {
            "-C" | "-c" => {
                it.next();
            }
            w if w.starts_with('-') => {}
            w => return Some(w),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn r(command: &str, exit_code: u8, fix: &str) -> Refusal {
        Refusal {
            session: "s".into(),
            command: command.into(),
            exit_code,
            fix: fix.into(),
            at: "t".into(),
            shown: false,
            lock: exit_code == HALTED,
        }
    }

    #[test]
    fn which_exits_stick() {
        for c in [4, 5, 6] {
            assert!(sticky("pr", c));
        }
        assert!(sticky("claim", 3));
        assert!(!sticky("pr", 3));
        assert!(!sticky("claim", 1));
        assert!(!sticky("claim", 2));
        assert!(!sticky("claim", 0));
    }

    #[test]
    fn only_kill_switch_halts_lock() {
        let l = |m: &str| locks(4, m, "wait");
        assert!(l("halt issue #3 by o is open: the owner stopped all work"));
        assert!(l("BOSUN_HALT=1: the owner stopped all work"));
        assert!(l("cannot read the bosun:halt issues: 500"));
        assert!(l("cannot read the BOSUN_HALT variable: 500"));
        let owners = "give the token Issues: read; bosun stays halted until it can read x";
        assert!(locks(
            4,
            "cannot read the owners on the default branch: 500",
            owners
        ));
        assert!(!l("main is red (#9); only mainfix tasks can be claimed"));
        assert!(!l("waves pause while process is red (#9)"));
        assert!(!l("api said: halted upstream"));
        assert!(!locks(5, "BOSUN_HALT=1", "wait"));
    }

    #[test]
    fn task_sessions_belong_to_the_caller() {
        assert!(mine("s1", "s1"));
        assert!(mine("s1/7", "s1"));
        assert!(!mine("s10", "s1"));
        assert!(!mine("s1", "s1/7"));
    }

    #[test]
    fn what_clears_a_refusal() {
        let d = json!({});
        assert!(clears(&r("pr", 6, "wait"), "pr", &d));
        assert!(clears(&r("pr", 5, "run `bosun hold 3 x`"), "hold", &d));
        assert!(clears(
            &r("pr", 5, "then bosun wave plan 9"),
            "wave plan",
            &d
        ));
        assert!(!clears(&r("pr", 5, "bosun hold"), "claim", &d));
        assert!(!clears(&r("pr", 5, "embosun hold"), "hold", &d));
        let lifted = json!({"halted": false, "main_red": null, "process_red": null});
        let red = json!({"halted": false, "main_red": 12});
        let still = json!({"halted": true, "main_red": null});
        assert!(clears(&r("claim", 4, "wait"), "status", &lifted));
        assert!(!clears(&r("claim", 4, "wait"), "status", &red));
        assert!(!clears(&r("claim", 4, "wait"), "status", &still));
        assert!(!clears(&r("pr", 6, "wait"), "status", &lifted));
    }

    #[test]
    fn context_names_command_exit_and_fix() {
        let c = context(&r("claim", 4, "wait for #3"));
        assert!(
            c.contains("`bosun claim`") && c.contains("exit 4 (halted)"),
            "{c}"
        );
        assert!(
            c.contains("wait for #3") && c.contains("bosun status"),
            "{c}"
        );
        let mut red = r("claim", 4, "wait for #9");
        red.lock = false;
        assert!(!context(&red).contains("halt lifted"));
        let c = context(&r("pr", 6, "x"));
        assert!(
            c.contains("exit 6 (limit)") && !c.contains("halt lifted"),
            "{c}"
        );
    }

    #[test]
    fn writes_are_told_from_reads() {
        let bash = |c: &str| is_write("Bash", &json!({"command": c}));
        for c in [
            "git push -u origin x",
            "cd a && git -C b commit -m x",
            "git -c user.name=x tag v1",
            "/usr/bin/git push",
            "gh pr create --draft",
            "echo; gh issue comment 3 -b x",
        ] {
            assert!(bash(c), "{c}");
        }
        for c in [
            "git status",
            "git log --grep push",
            "gh pr view 3",
            "bosun status",
            "cargo test",
            "echo git",
        ] {
            assert!(!bash(c), "{c}");
        }
        assert!(is_write("Edit", &json!({})));
        assert!(is_write("mcp__github__create_pull_request", &json!({})));
        assert!(is_write("mcp__github__merge_pull_request", &json!({})));
        assert!(!is_write("mcp__github__get_file_contents", &json!({})));
        assert!(!is_write("mcp__github__pull_request_read", &json!({})));
        assert!(!is_write("Read", &json!({})));
    }
}
