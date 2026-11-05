//! repeat-lock: the same Bash command run again and again by one session (spec §4). Pure.

use serde::{Deserialize, Serialize};

/// Runs in a row at which the hook warns, and from which it denies.
pub const WARN_AT: u64 = 3;
pub const DENY_AT: u64 = 5;

/// The counter of one session, agent and directory: its last command and the runs in a row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repeat {
    pub command: String,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Warn(String),
    Deny(String),
}

/// The counter after one more call: the same trimmed command adds one, another restarts at one.
pub fn repeat_step(prev: Option<Repeat>, command: &str) -> Repeat {
    let command = command.trim().to_string();
    let prev = prev.filter(|p| p.command == command);
    let count = prev.map_or(1, |p| p.count.saturating_add(1));
    Repeat { command, count }
}

/// Read-only polls that orchestrators re-run on purpose (the Bash cap is 600 s).
const POLLS: &[&str] = &[
    "bosun wait",
    "bosun status",
    "bosun heartbeat",
    "bosun doctor",
    "bosun ci-log",
    "bosun review-pack",
    "sleep",
    "gh pr checks",
    "gh pr view",
    "gh pr status",
    "gh run view",
    "gh run watch",
    "gh run list",
    "git status",
    "git log",
    "git fetch",
    "git diff",
];

/// A poll alone (`2>&1` at most): no operators, substitutions or `--output`. The repeat-lock
/// ignores it.
pub fn is_poll(command: &str) -> bool {
    let command = command.trim().trim_end_matches("2>&1");
    let mut w: Vec<&str> = command.split_whitespace().collect();
    let ops = command.contains([';', '&', '|', '<', '>', '`', '$', '(', '\n', '\\']);
    if ops || command.contains("--output") || w.is_empty() {
        return false;
    }
    w[0] = w[0].rsplit('/').next().unwrap_or(w[0]);
    let line = w.join(" ");
    POLLS
        .iter()
        .any(|p| line == *p || line.starts_with(&format!("{p} ")))
}

/// The counter key of a session, agent and directory: 16 hex digits (FNV-1a), a safe file name.
pub fn repeat_key(session: &str, agent: &str, dir: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in [session, agent, dir].join("\0").bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// The hook's answer after `count` runs in a row: a warning at the third, a deny from the fifth.
pub fn repeat_verdict(count: u64) -> Option<Verdict> {
    let ran = format!("repeat-lock: this exact command ran {count} times in a row;");
    match count {
        DENY_AT.. => Some(Verdict::Deny(format!(
            "{ran} retrying does not help, change the approach (read the error, try another \
             command) or ask the owner"
        ))),
        WARN_AT => Some(Verdict::Warn(format!(
            "{ran} change the approach instead of retrying (the {DENY_AT}th run is refused)"
        ))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_same_command_and_resets_on_change() {
        let a = repeat_step(None, " ls ");
        assert_eq!(a.count, 1);
        let b = repeat_step(Some(a), "ls");
        assert_eq!(b.count, 2);
        assert_eq!(repeat_step(Some(b), "pwd").count, 1);
    }

    #[test]
    fn keys_differ_per_session_agent_and_dir() {
        let k = repeat_key("s", "", "/r");
        assert_eq!(k.len(), 16);
        assert_eq!(k, repeat_key("s", "", "/r"));
        assert_ne!(k, repeat_key("t", "", "/r"));
        assert_ne!(k, repeat_key("s", "a", "/r"));
        assert_ne!(k, repeat_key("s", "", "/r/w"));
        assert_ne!(repeat_key("s", "a/b", ""), repeat_key("s", "a", "/b"));
    }

    #[test]
    fn polls_are_read_only_commands_only() {
        for c in [
            "bosun wait --child S",
            " bosun status ",
            "/usr/bin/bosun heartbeat",
            "bosun doctor",
            "sleep 60",
            "gh pr checks 5",
            "gh pr view 5 --json state",
            "gh run watch 9",
            "gh run list",
            "git status",
            "git log -3",
            "git fetch origin",
            "git diff --stat",
            "bosun wait --pr 5 2>&1",
        ] {
            assert!(is_poll(c), "{c}");
        }
        for c in [
            "cargo test",
            "bosun claim T1",
            "bosun release T1",
            "bosun risk-check 5",
            "bosun inbox",
            "git diff --output=x",
            "gh pr merge 5",
            "git push",
            "bosun wait; rm x",
            "bosun status | tee f",
            "sleep 5 && cargo test",
            "bosun wait > f",
            "git status $(x)",
            "gh pr checks 5\nrm x",
            "",
            "bosun",
        ] {
            assert!(!is_poll(c), "{c}");
        }
    }

    #[test]
    fn verdicts() {
        let kind = |n| match repeat_verdict(n) {
            Some(Verdict::Warn(t)) => format!("warn {t}"),
            Some(Verdict::Deny(t)) => format!("deny {t}"),
            None => String::new(),
        };
        let v: Vec<_> = (1..=6).map(kind).collect();
        assert!(
            v[0].is_empty() && v[1].is_empty() && v[3].is_empty(),
            "{v:?}"
        );
        assert!(v[2].starts_with("warn repeat-lock: this exact command ran 3 times"));
        assert!(v[4].starts_with("deny repeat-lock") && v[5].starts_with("deny repeat-lock"));
    }
}
