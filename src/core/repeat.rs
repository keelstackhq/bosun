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

pub fn warn_text(count: u64) -> String {
    format!(
        "repeat-lock: this exact command ran {count} times in a row; change the approach \
         instead of retrying (the {DENY_AT}th run is refused)"
    )
}

pub fn deny_text(count: u64) -> String {
    format!(
        "repeat-lock: this exact command ran {count} times in a row; retrying does not help, \
         change the approach (read the error, try another command) or ask the owner"
    )
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
    fn verdicts() {
        let v: Vec<_> = (1..=6).map(repeat_verdict).collect();
        use Verdict::*;
        assert_eq!(v, [None, None, Some(Warn), None, Some(Deny), Some(Deny)]);
    }
}
