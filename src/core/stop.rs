//! Stop rules and model tier escalation (spec §4). Pure: the caller reads the notes.
//! An `escalate` record restarts the count: only `failure` records after it are looked at.

use super::records::{Note, Record, parse_record};

/// Model tiers, cheapest first.
pub const TIERS: [&str; 3] = ["haiku", "sonnet", "opus"];
/// Equal failures in a row that stop a task.
pub const SAME_FAILURE_LIMIT: usize = 3;

/// Why a task stopped.
#[derive(Debug, Clone, PartialEq)]
pub struct Stop {
    /// `same-failure` or `empty-fix`.
    pub rule: &'static str,
    pub signature: String,
    /// Equal failures in a row at the end of the list.
    pub streak: usize,
}

/// The index in `TIERS` of a model id or alias: the tier word it contains
/// (`claude-sonnet-5-5` is sonnet). An unknown model counts as `build`; if `build` is unknown
/// too, as sonnet.
pub fn tier_index(model: &str, build: &str) -> usize {
    let find = |m: &str| {
        let m = m.to_ascii_lowercase();
        TIERS.iter().position(|t| m.contains(t))
    };
    find(model).or_else(|| find(build)).unwrap_or(1)
}

/// The tier above `model`, or `None` for opus (escalation is used up).
pub fn next_tier(model: &str, build: &str) -> Option<&'static str> {
    TIERS.get(tier_index(model, build) + 1).copied()
}

/// The tier in use: the `to` of the newest `escalate` record, else `default_build_model`.
pub fn current_tier(notes: &[Note], default_build_model: &str) -> String {
    notes
        .iter()
        .rev()
        .find_map(|n| match parse_record(&n.body) {
            Some(Record::Escalate { to, .. }) => Some(to),
            _ => None,
        })
        .unwrap_or_else(|| default_build_model.to_string())
}

/// The `failure` records after the newest `escalate` record: (signature, changed lines).
fn recent_failures(notes: &[Note]) -> Vec<(String, Option<u64>)> {
    let mut out = Vec::new();
    for n in notes.iter().rev() {
        match parse_record(&n.body) {
            Some(Record::Escalate { .. }) => break,
            Some(Record::Failure {
                signature,
                changed_lines,
                ..
            }) => out.push((signature, changed_lines)),
            _ => {}
        }
    }
    out.reverse();
    out
}

/// The newest signature and how many equal failures end the list (0 when there is none).
pub fn streak(notes: &[Note]) -> (Option<String>, usize) {
    let failures = recent_failures(notes);
    let Some((last, _)) = failures.last() else {
        return (None, 0);
    };
    let n = failures.iter().rev().take_while(|(s, _)| s == last).count();
    (Some(last.clone()), n)
}

/// The stop rule that now applies, if any. `empty-fix` (newest failure changed 0 lines) wins
/// over `same-failure` (the newest 3 failures share one signature).
pub fn evaluate(notes: &[Note]) -> Option<Stop> {
    let failures = recent_failures(notes);
    let (signature, changed) = failures.last()?;
    let (_, n) = streak(notes);
    let rule = if *changed == Some(0) {
        "empty-fix"
    } else if n >= SAME_FAILURE_LIMIT {
        "same-failure"
    } else {
        return None;
    };
    Some(Stop {
        rule,
        signature: signature.clone(),
        streak: n,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fail(id: u64, sig: &str, lines: Option<u64>) -> Note {
        let r = Record::Failure {
            session: "s".into(),
            signature: sig.into(),
            changed_lines: lines,
        };
        Note::new(id, r.to_line())
    }

    fn esc(id: u64, to: &str) -> Note {
        let r = Record::Escalate {
            session: "s".into(),
            from: "sonnet".into(),
            to: to.into(),
            reason: "same-failure".into(),
        };
        Note::new(id, r.to_line())
    }

    #[test]
    fn tiers_go_up_by_one() {
        assert_eq!(next_tier("haiku", "sonnet"), Some("sonnet"));
        assert_eq!(next_tier("claude-sonnet-5-5", "sonnet"), Some("opus"));
        assert_eq!(next_tier("Opus", "sonnet"), None);
        // Unknown model counts as the build tier.
        assert_eq!(next_tier("gpt-x", "haiku"), Some("sonnet"));
        assert_eq!(next_tier("gpt-x", "odd"), Some("opus"));
    }

    #[test]
    fn three_equal_failures_stop() {
        let notes = vec![fail(1, "a", None), fail(2, "a", None)];
        assert_eq!(evaluate(&notes), None);
        let mut notes = notes;
        notes.push(fail(3, "a", Some(4)));
        let stop = evaluate(&notes).unwrap();
        assert_eq!(
            (stop.rule, stop.signature.as_str(), stop.streak),
            ("same-failure", "a", 3)
        );
    }

    #[test]
    fn another_signature_breaks_the_streak() {
        let notes = vec![fail(1, "a", None), fail(2, "b", None), fail(3, "a", None)];
        assert_eq!(evaluate(&notes), None);
        assert_eq!(streak(&notes), (Some("a".into()), 1));
    }

    #[test]
    fn zero_changed_lines_stops_at_once() {
        let stop = evaluate(&[fail(1, "a", Some(0))]).unwrap();
        assert_eq!((stop.rule, stop.streak), ("empty-fix", 1));
    }

    #[test]
    fn an_escalation_restarts_the_count() {
        let notes = vec![
            fail(1, "a", None),
            fail(2, "a", None),
            esc(3, "opus"),
            fail(4, "a", None),
        ];
        assert_eq!(evaluate(&notes), None);
        assert_eq!(current_tier(&notes, "sonnet"), "opus");
        assert_eq!(current_tier(&notes[..2], "sonnet"), "sonnet");
    }
}
