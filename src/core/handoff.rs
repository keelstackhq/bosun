//! A root's handoff (spec §5.1): the lines it leaves for the next root. Pure.

use crate::core::records::{Note, Record, parse_record};
use crate::core::text::{has_hidden, inert};
use crate::core::verdict::{Verdict, comment_json};

/// At most this many lines in one handoff.
pub const MAX_LINES: usize = 40;
/// At most this many characters in one line.
pub const MAX_LINE: usize = 300;
/// The kinds a line may start with: `<kind>: <text>`.
pub const KINDS: &[&str] = &["wave", "pr", "lease", "next", "blocker"];

/// Whether one trimmed line is a `<kind>: <text>` line that is safe to show.
pub fn check_line(line: &str) -> Result<(), String> {
    let kind = line.split_once(": ").map(|(k, _)| k);
    if !kind.is_some_and(|k| KINDS.contains(&k)) {
        return Err(format!(
            "must start with one of {} followed by `: `",
            KINDS.join(", ")
        ));
    }
    if has_hidden(line) {
        return Err("has control or hidden characters".into());
    }
    if line.chars().count() > MAX_LINE {
        return Err(format!("is longer than {MAX_LINE} characters"));
    }
    Ok(())
}

/// A line bosun builds itself: hidden characters as spaces, runs of spaces squeezed, cut to
/// [`MAX_LINE`] characters.
pub fn clean_line(line: &str) -> String {
    let spaced: String = line
        .chars()
        .map(|c| {
            if has_hidden(c.encode_utf8(&mut [0; 4])) {
                ' '
            } else {
                c
            }
        })
        .collect();
    spaced
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_LINE)
        .collect()
}

/// The lines of a handoff file: trimmed, blank lines skipped. The error names the first bad line.
pub fn parse_lines(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        check_line(line).map_err(|e| format!("line {} {e}", i + 1))?;
        out.push(line.to_string());
    }
    Ok(out)
}

/// Whether `session` is a root that may hand off: its id has no `/` (no per-task agent), it
/// joined with no parent and a non-spawned origin, and it never joined with a parent. Reads the
/// join records directly, so a root that left is still one. All sessions share one GitHub
/// writer, so the registry's `join` is the only thing that tells roots from children.
pub fn is_root(notes: &[Note], session: &str) -> bool {
    let (mut root, mut child) = (false, false);
    for n in notes {
        if let Some(Record::Join {
            session: s,
            origin,
            parent,
            ..
        }) = parse_record(&n.body)
            && s == session
        {
            child |= parent.is_some();
            root |= parent.is_none() && origin != "spawned";
        }
    }
    root && !child && !session.contains('/')
}

/// The old root of the newest relay that named `me` as the new root, if that root is one.
pub fn lineage(notes: &[Note], me: &str) -> Option<String> {
    notes
        .iter()
        .rev()
        .find_map(|n| match parse_record(&n.body) {
            Some(Record::Relay { session, root, .. }) if root == me && is_root(notes, &session) => {
                Some(session)
            }
            _ => None,
        })
}

/// The newest well-formed handoff of a root, from trusted notes (oldest first). Of the `prefer`
/// sessions (the lineage root and the caller), the newest handoff wins; with none of theirs,
/// the newest root's.
pub fn newest_handoff(notes: &[Note], prefer: &[&str]) -> Option<(u64, Record)> {
    let mut valid = notes
        .iter()
        .rev()
        .filter_map(|n| match parse_record(&n.body) {
            Some(Record::Handoff { session, at, lines })
                if is_root(notes, &session)
                    && lines.len() <= MAX_LINES
                    && lines.iter().all(|l| check_line(l).is_ok()) =>
            {
                Some((n.id, Record::Handoff { session, at, lines }))
            }
            _ => None,
        });
    let all: Vec<_> = valid.by_ref().collect();
    let named = all.iter().find(|(_, r)| prefer.contains(&r.session()));
    named.or(all.first()).cloned()
}

/// The parent a join record names for `session`, newest record last.
pub fn parent_of(notes: &[Note], session: &str) -> Option<String> {
    notes
        .iter()
        .filter_map(|n| match parse_record(&n.body) {
            Some(Record::Join {
                session: s, parent, ..
            }) if s == session => Some(parent),
            _ => None,
        })
        .next_back()
        .flatten()
}

/// `file:line claim` of every minor finding in the newest verdict on `head` among the PR
/// comment bodies (oldest first). Several verdicts: the last one wins.
pub fn minors<'a>(bodies: impl IntoIterator<Item = &'a str>, head: &str) -> Vec<String> {
    let verdict = bodies
        .into_iter()
        .filter_map(|b| comment_json(b).and_then(|j| Verdict::parse(j).ok()))
        .filter(|v| v.head_sha == head)
        .last();
    verdict
        .map(|v| {
            v.findings
                .iter()
                .filter(|f| f.severity == "minor")
                .map(|f| {
                    let at = f.line.map_or(f.file.clone(), |l| format!("{}:{l}", f.file));
                    format!("{at} {}", inert(&f.claim, 120))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The message a child sends its parent when its PR is ready (at most about 1,200 characters).
pub fn pr_message(from: &str, pr: u64, head: &str, risk: &[String], minors: &[String]) -> String {
    fn list(items: &[String], keep: usize) -> String {
        if items.is_empty() {
            return "none".into();
        }
        let shown = items
            .iter()
            .take(keep)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        match items.len().saturating_sub(keep) {
            0 => shown,
            more => format!("{shown}; +{more} more"),
        }
    }
    format!(
        "From {from}: PR #{pr} is ready for review. head {head}. risk paths: {}. review minors: {}.",
        list(risk, 8),
        list(minors, 5)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: u64, session: &str, line: &str) -> Note {
        let r = Record::Handoff {
            session: session.into(),
            at: "t".into(),
            lines: vec![line.into()],
        };
        Note::new(id, r.to_line())
    }

    fn join(id: u64, session: &str, parent: Option<&str>, origin: &str) -> Note {
        let parent = parent.map_or(String::new(), |p| format!(",\"parent\":\"{p}\""));
        Note::new(
            id,
            format!(
                "{{\"bosun\":\"join\",\"session\":\"{session}\",\"origin\":\"{origin}\"{parent},\"depth\":1,\"expires\":\"2026-10-22T11:20:00Z\"}}"
            ),
        )
    }

    #[test]
    fn lines_are_trimmed_and_blank_lines_skipped() {
        let got = parse_lines("  next: a\n\n pr: #1 x y \n").unwrap();
        assert_eq!(got, ["next: a", "pr: #1 x y"]);
    }

    #[test]
    fn a_bad_kind_names_the_line() {
        let e = parse_lines("next: a\n\ntodo: b").unwrap_err();
        assert!(e.starts_with("line 3 "), "{e}");
        assert!(parse_lines("next:a").is_err());
    }

    #[test]
    fn hidden_and_long_lines_are_refused() {
        assert!(
            parse_lines("next: a\u{202e}b")
                .unwrap_err()
                .contains("hidden")
        );
        let long = format!("next: {}", "x".repeat(MAX_LINE));
        assert!(parse_lines(&long).unwrap_err().contains("longer"));
    }

    #[test]
    fn the_record_line_shape_round_trips() {
        let r = Record::Handoff {
            session: "s".into(),
            at: "2026-10-22T11:00:00Z".into(),
            lines: vec!["next: a".into()],
        };
        assert_eq!(
            r.to_line(),
            r#"{"bosun":"handoff","session":"s","at":"2026-10-22T11:00:00Z","lines":["next: a"]}"#
        );
        assert_eq!(parse_record(&r.to_line()), Some(r.clone()));
        assert_eq!(r.session(), "s");
    }

    #[test]
    fn only_joined_parentless_unspawned_slashless_sessions_are_roots() {
        let notes = vec![
            join(1, "root", None, "local"),
            join(2, "child", Some("root"), "local"),
            join(3, "spawn", None, "spawned"),
            join(4, "root/T7", None, "local"),
            join(5, "both", None, "local"),
            join(6, "both", Some("root"), "local"),
        ];
        let is = |s| is_root(&notes, s);
        assert!(is("root"));
        assert!(!is("child") && !is("spawn") && !is("root/T7") && !is("both") && !is("nobody"));
    }

    #[test]
    fn the_newest_valid_root_handoff_wins_and_a_named_lineage_is_preferred() {
        let notes = vec![
            join(1, "root", None, "local"),
            join(2, "other", None, "local"),
            join(3, "child", Some("root"), "local"),
            note(4, "root", "next: old"),
            note(5, "root", "next: new"),
            note(6, "other", "next: other"),
            note(7, "child", "next: child"),
            note(8, "root", "todo: bad kind"),
            note(9, "ghost", "next: unjoined"),
        ];
        assert_eq!(newest_handoff(&notes, &[]).unwrap().0, 6);
        assert_eq!(newest_handoff(&notes, &["root"]).unwrap().0, 5);
        assert_eq!(newest_handoff(&notes, &["child"]).unwrap().0, 6);
        assert_eq!(newest_handoff(&notes, &["root", "other"]).unwrap().0, 6);
        assert!(newest_handoff(&notes[2..4], &[]).is_none());
        let many = Record::Handoff {
            session: "root".into(),
            at: "t".into(),
            lines: vec!["next: a".into(); MAX_LINES + 1],
        };
        let mut n = notes[..4].to_vec();
        n.push(Note::new(10, many.to_line()));
        assert_eq!(newest_handoff(&n, &[]).unwrap().0, 4);
    }

    #[test]
    fn lineage_is_the_old_root_of_the_newest_relay_naming_me() {
        let relay = |id, old: &str, new: &str| {
            let r = Record::Relay {
                session: old.into(),
                root: new.into(),
                at: "t".into(),
            };
            Note::new(id, r.to_line())
        };
        let notes = vec![
            join(1, "a", None, "local"),
            join(2, "b", None, "relay"),
            join(3, "c", None, "relay"),
            relay(4, "a", "b"),
            relay(5, "b", "c"),
            relay(6, "ghost", "c"),
            relay(7, "child", "d"),
            join(8, "child", Some("a"), "local"),
        ];
        assert_eq!(lineage(&notes, "b").as_deref(), Some("a"));
        assert_eq!(lineage(&notes, "c").as_deref(), Some("b"));
        assert_eq!(lineage(&notes, "d"), None);
        assert_eq!(lineage(&notes, "a"), None);
        assert!(is_root(&notes, "b") && is_root(&notes, "c"));
    }

    #[test]
    fn built_lines_lose_hidden_characters_and_extra_spaces() {
        assert_eq!(
            clean_line("lease: #1  s \u{202e}a  until x"),
            "lease: #1 s a until x"
        );
        assert_eq!(clean_line(&"x".repeat(500)).chars().count(), MAX_LINE);
    }
}
