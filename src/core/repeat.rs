//! repeat-lock: the same Bash command run again and again by one session (spec §4). Pure.

/// Calls in a row at which the hook warns.
pub const WARN_AT: u64 = 3;
/// Calls in a row from which the hook denies.
pub const DENY_AT: u64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repeat {
    pub session: String,
    pub command: String,
    pub count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Warn,
    Deny,
}

/// The counter after one more call: the same session and trimmed command add one, anything
/// else restarts at one.
pub fn repeat_step(prev: Option<&Repeat>, session: &str, command: &str) -> Repeat {
    let command = command.trim();
    let count = match prev {
        Some(p) if p.session == session && p.command == command => p.count.saturating_add(1),
        _ => 1,
    };
    Repeat {
        session: session.into(),
        command: command.into(),
        count,
    }
}

/// Read-only commands (name, leading arguments) that orchestrators re-run on purpose.
#[rustfmt::skip]
const POLLS: &[(&str, &[&str])] = &[
    ("bosun", &["wait", "status", "heartbeat", "inbox", "doctor", "ci-log", "risk-check", "review-pack"]),
    ("sleep", &[]), ("git", &["status", "log", "fetch", "diff"]),
    ("gh", &["pr checks", "pr view", "pr status", "run view", "run watch", "run list"]),
];

/// Read-only polls that orchestrators re-run on purpose (the Bash cap is 600 s): a plain
/// command, no operators or substitutions. The repeat-lock ignores them.
pub fn is_poll(command: &str) -> bool {
    let command = command.trim().trim_end_matches("2>&1");
    if command.contains([';', '&', '|', '<', '>', '`', '$', '(', '\n', '\\']) {
        return false;
    }
    let w: Vec<&str> = command.split_whitespace().collect();
    let name = w.first().map_or("", |n| n.rsplit('/').next().unwrap_or(n));
    let args = w.iter().skip(1).copied().collect::<Vec<_>>().join(" ");
    POLLS.iter().any(|(n, subs)| {
        *n == name
            && (subs.is_empty() && !args.is_empty()
                || subs
                    .iter()
                    .any(|s| args == *s || args.starts_with(&format!("{s} "))))
    })
}

/// The counter key of a session, agent and directory: 16 hex digits (FNV-1a), a safe file name.
pub fn repeat_key(session: &str, agent: &str, dir: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in [session, agent, dir].join("\0").bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Warn at the third call in a row, deny from the fifth.
pub fn repeat_verdict(count: u64) -> Option<Verdict> {
    match count {
        DENY_AT.. => Some(Verdict::Deny),
        WARN_AT => Some(Verdict::Warn),
        _ => None,
    }
}

#[rustfmt::skip]
pub fn warn_text(count: u64) -> String {
    format!("repeat-lock: this exact command ran {count} times in a row; change the approach instead of retrying (the {DENY_AT}th run is refused)")
}

#[rustfmt::skip]
pub fn deny_text(count: u64) -> String {
    format!("repeat-lock: this exact command ran {count} times in a row; retrying does not help, change the approach (read the error, try another command) or ask the owner")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_same_command_and_resets_on_change() {
        let a = repeat_step(None, "s", " ls ");
        assert_eq!(a.count, 1);
        let b = repeat_step(Some(&a), "s", "ls");
        assert_eq!(b.count, 2);
        assert_eq!(repeat_step(Some(&b), "s", "pwd").count, 1);
        assert_eq!(repeat_step(Some(&b), "t", "ls").count, 1);
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
            "bosun inbox",
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
        let v: Vec<_> = (1..=6).map(repeat_verdict).collect();
        use Verdict::*;
        assert_eq!(v, [None, None, Some(Warn), None, Some(Deny), Some(Deny)]);
    }
}
