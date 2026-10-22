//! Machine records in issue comments: one line of JSON that starts with `{"bosun":"<type>"`.
//! The first such line of a comment body is its record; other text is for people.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "bosun", rename_all = "kebab-case")]
pub enum Record {
    /// A claim on an issue's touch paths (spec §4).
    Lease {
        session: String,
        origin: String,
        expires: String,
        touch: Vec<String>,
    },
    /// The end of a session's lease on this issue: `released` or `reaped`.
    LeaseEnd { session: String, reason: String },
    /// A session in the registry.
    Join {
        session: String,
        origin: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        area: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<String>,
        depth: u32,
        expires: String,
    },
    /// A session left the registry (released, reaped, or done).
    Leave { session: String, reason: String },
    /// A child's result for its parent (spec D14).
    Result {
        session: String,
        outcome: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pr: Option<u64>,
    },
    /// Money spent by a session on a task.
    Spend {
        session: String,
        usd: f64,
        at: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        issue: Option<u64>,
    },
    /// A question to the owner.
    Ask { session: String, question: String },
    /// A hit on a lesson.
    LessonHit { session: String, id: String },
    /// A posted review verdict on a PR head (spec §7); the next review is a delta from it.
    Verdict {
        session: String,
        head_sha: String,
        approved: bool,
    },
}

impl Record {
    /// The comment line: compact JSON, `bosun` key first.
    pub fn to_line(&self) -> String {
        serde_json::to_string(self).expect("records serialize")
    }

    pub fn session(&self) -> &str {
        match self {
            Record::Lease { session, .. }
            | Record::LeaseEnd { session, .. }
            | Record::Join { session, .. }
            | Record::Leave { session, .. }
            | Record::Result { session, .. }
            | Record::Spend { session, .. }
            | Record::Ask { session, .. }
            | Record::LessonHit { session, .. }
            | Record::Verdict { session, .. } => session,
        }
    }
}

/// The record of a comment body, if it has one. Unknown types and bad JSON are `None`.
pub fn parse_record(body: &str) -> Option<Record> {
    body.lines()
        .map(str::trim)
        .find(|l| l.starts_with(r#"{"bosun":"#))
        .and_then(|l| serde_json::from_str(l).ok())
}

/// `body` with its record line replaced by `record`; other lines stay. A body without a
/// record line becomes just the record.
pub fn replace_record(body: &str, record: &Record) -> String {
    let mut done = false;
    let lines: Vec<String> = body
        .lines()
        .map(|l| {
            if !done && l.trim().starts_with(r#"{"bosun":"#) {
                done = true;
                record.to_line()
            } else {
                l.to_string()
            }
        })
        .collect();
    if done {
        lines.join("\n")
    } else {
        record.to_line()
    }
}

/// A comment as core sees it: id, body and creation time, oldest first in any list.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    pub id: u64,
    pub body: String,
    /// When GitHub created the comment; `Timestamp(0)` when unknown.
    pub created_at: crate::time::Timestamp,
}

impl Note {
    /// A note without a known creation time.
    pub fn new(id: u64, body: impl Into<String>) -> Note {
        Note {
            id,
            body: body.into(),
            created_at: crate::time::Timestamp(0),
        }
    }
}

/// Sum of the `spend` records in `notes` whose `at` is not before `since` (all when `None`).
/// A record with an unreadable time counts only when `since` is `None`.
pub fn spend_total(notes: &[Note], since: Option<crate::time::Timestamp>) -> f64 {
    notes
        .iter()
        .filter_map(|n| match parse_record(&n.body) {
            Some(Record::Spend { usd, at, .. }) => {
                let ok =
                    since.is_none_or(|s| crate::time::parse_rfc3339(&at).is_some_and(|t| t >= s));
                ok.then_some(usd)
            }
            _ => None,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_line_matches_the_spec_shape() {
        let r = Record::Lease {
            session: "session_1".into(),
            origin: "spawned".into(),
            expires: "2026-10-22T10:30:00Z".into(),
            touch: vec!["src/core/lease.rs".into()],
        };
        assert_eq!(
            r.to_line(),
            r#"{"bosun":"lease","session":"session_1","origin":"spawned","expires":"2026-10-22T10:30:00Z","touch":["src/core/lease.rs"]}"#
        );
        assert_eq!(parse_record(&r.to_line()), Some(r));
    }

    #[test]
    fn human_text_around_the_record_is_ignored() {
        let body = "Claimed by bosun.\n\n{\"bosun\":\"lease-end\",\"session\":\"s\",\"reason\":\"reaped\"}\nbye";
        assert_eq!(
            parse_record(body),
            Some(Record::LeaseEnd {
                session: "s".into(),
                reason: "reaped".into()
            })
        );
    }

    #[test]
    fn replace_keeps_human_text() {
        let old = Record::Leave {
            session: "s".into(),
            reason: "a".into(),
        };
        let new = Record::Leave {
            session: "s".into(),
            reason: "b".into(),
        };
        let body = format!("hello\n{}\nbye", old.to_line());
        assert_eq!(
            replace_record(&body, &new),
            format!("hello\n{}\nbye", new.to_line())
        );
        assert_eq!(replace_record("text only", &new), new.to_line());
    }

    #[test]
    fn unknown_or_broken_records_are_none() {
        assert_eq!(parse_record("just text"), None);
        assert_eq!(parse_record(r#"{"bosun":"nope","session":"s"}"#), None);
        assert_eq!(parse_record(r#"{"bosun":"lease","session":"s"}"#), None);
        assert_eq!(parse_record(r#"{"other":"lease"}"#), None);
    }
}
