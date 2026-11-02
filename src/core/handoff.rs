//! A root's handoff (spec §5.1): the lines it leaves for the next root. Pure.

use crate::core::records::{Note, Record, parse_record};
use crate::core::text::has_hidden;

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

/// The newest handoff of the root lineage: the last well-formed `handoff` record whose session
/// never joined with a parent (a child's handoff is not the root's, also after it left).
/// Notes are oldest first and must be trusted ones.
pub fn newest_handoff(notes: &[Note]) -> Option<(u64, Record)> {
    notes
        .iter()
        .rev()
        .find_map(|n| match parse_record(&n.body) {
            Some(Record::Handoff { session, at, lines })
                if !is_child(notes, &session)
                    && lines.len() <= MAX_LINES
                    && lines.iter().all(|l| check_line(l).is_ok()) =>
            {
                Some((n.id, Record::Handoff { session, at, lines }))
            }
            _ => None,
        })
}

/// Whether `session` ever joined with a parent in these notes.
pub fn is_child(notes: &[Note], session: &str) -> bool {
    notes.iter().any(|n| {
        matches!(parse_record(&n.body),
            Some(Record::Join { session: s, parent: Some(_), .. }) if s == session)
    })
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

    fn join(id: u64, session: &str, parent: &str) -> Note {
        Note::new(
            id,
            format!(
                "{{\"bosun\":\"join\",\"session\":\"{session}\",\"origin\":\"local\",\"parent\":\"{parent}\",\"depth\":2,\"expires\":\"2026-10-22T11:20:00Z\"}}"
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
    fn the_newest_root_handoff_wins_and_a_childs_is_skipped() {
        let notes = vec![
            note(1, "root", "next: old"),
            note(2, "root", "next: new"),
            join(3, "child", "root"),
            note(4, "child", "next: child"),
        ];
        let (id, r) = newest_handoff(&notes).unwrap();
        assert_eq!(id, 2);
        assert!(matches!(r, Record::Handoff { lines, .. } if lines == ["next: new"]));
        assert!(newest_handoff(&notes[2..]).is_none());
    }

    #[test]
    fn a_child_who_left_is_still_a_child_and_bad_records_are_skipped() {
        let mut notes = vec![
            note(1, "root", "next: ok"),
            join(2, "child", "root"),
            note(3, "child", "next: child"),
            note(4, "root", "todo: bad kind"),
        ];
        let (id, _) = newest_handoff(&notes).unwrap();
        assert_eq!(id, 1);
        assert!(is_child(&notes, "child") && !is_child(&notes, "root"));
        let many = Record::Handoff {
            session: "root".into(),
            at: "t".into(),
            lines: vec!["next: a".into(); MAX_LINES + 1],
        };
        notes.push(Note::new(5, many.to_line()));
        assert_eq!(newest_handoff(&notes).unwrap().0, 1);
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
