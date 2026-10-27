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
        /// The Slack thread that started this work (`bosun spawn --slack-thread`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread_ts: Option<String>,
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
        /// The verdict's confidence; a record without it never counts toward two verdicts.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        confidence: Option<f64>,
        /// Major findings in the verdict; a record without it never lets a delegate approve.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        major: Option<u32>,
    },
    /// A failed fix round: the failure signature and the lines it changed (spec §4 stop rules).
    Failure {
        session: String,
        signature: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        changed_lines: Option<u64>,
    },
    /// A model tier escalation after a stop rule; it restarts the failure count.
    Escalate {
        session: String,
        from: String,
        to: String,
        reason: String,
    },
    /// The same failure signature seen again; `issue` is the task where it was seen (spec §6).
    LessonSeen {
        session: String,
        signature: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        issue: Option<u64>,
    },
    /// A Slack post for `key` was made (spec §10, `bosun notify --ack`).
    Notified { key: String, at: String },
    /// An owner approved PRs (and posted `main-open`) with `bosun approve`; no session id.
    Approve {
        prs: Vec<PrHead>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        main_open: Vec<PrHead>,
        at: String,
    },
    /// The PR that `bosun pr --auto-merge` queued for this task; `reap` reads it directly.
    TaskPr { session: String, pr: u64 },
    /// An open PR that became conflicting with its base (`mergeable_state` dirty), posted by
    /// `bosun main-health`. One per head and base sha pair.
    Conflict {
        pr: u64,
        head_sha: String,
        base_sha: String,
    },
    /// An owner's answer to an `ask`, given through Slack. The text is not in the record.
    Answer {
        session: String,
        slack_user: String,
        at: String,
    },
    /// A human step found mid-run (`bosun hold`): `id` is the `X<n>` box that holds the task
    /// until the owner ticks it; `on` is `computer` or `phone`.
    Hold {
        session: String,
        id: String,
        step: String,
        on: String,
    },
}

/// A PR and the head sha an owner acted on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrHead {
    pub pr: u64,
    pub head_sha: String,
}

impl Record {
    /// The comment line: compact JSON, `bosun` key first.
    pub fn to_line(&self) -> String {
        serde_json::to_string(self).expect("records serialize")
    }

    /// The session that wrote the record; `""` for a `notified` record, which has none.
    pub fn session(&self) -> &str {
        match self {
            Record::Notified { .. } | Record::Approve { .. } | Record::Conflict { .. } => "",
            Record::Lease { session, .. }
            | Record::LeaseEnd { session, .. }
            | Record::Join { session, .. }
            | Record::Leave { session, .. }
            | Record::Result { session, .. }
            | Record::Spend { session, .. }
            | Record::Ask { session, .. }
            | Record::LessonHit { session, .. }
            | Record::Verdict { session, .. }
            | Record::Failure { session, .. }
            | Record::Escalate { session, .. }
            | Record::LessonSeen { session, .. }
            | Record::Answer { session, .. }
            | Record::Hold { session, .. }
            | Record::TaskPr { session, .. } => session,
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
/// A record with an unreadable time counts only when `since` is `None`. A negative or
/// non-finite amount is ignored: records are trusted from any commenter, and a forged
/// negative one must not lower the spend.
pub fn spend_total(notes: &[Note], since: Option<crate::time::Timestamp>) -> f64 {
    notes
        .iter()
        .filter_map(|n| match parse_record(&n.body) {
            Some(Record::Spend { usd, at, .. }) if usd.is_finite() && usd >= 0.0 => {
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
            thread_ts: None,
        };
        assert_eq!(
            r.to_line(),
            r#"{"bosun":"lease","session":"session_1","origin":"spawned","expires":"2026-10-22T10:30:00Z","touch":["src/core/lease.rs"]}"#
        );
        assert_eq!(parse_record(&r.to_line()), Some(r));
    }

    #[test]
    fn a_lease_without_thread_ts_still_parses() {
        let old = r#"{"bosun":"lease","session":"s","origin":"local","expires":"2026-10-22T10:30:00Z","touch":["a"]}"#;
        assert!(matches!(
            parse_record(old),
            Some(Record::Lease {
                thread_ts: None,
                ..
            })
        ));
    }

    #[test]
    fn verdict_confidence_is_optional_and_old_records_parse() {
        let r = Record::Verdict {
            session: "s".into(),
            head_sha: "h".into(),
            approved: true,
            confidence: Some(0.95),
            major: Some(1),
        };
        assert_eq!(
            r.to_line(),
            r#"{"bosun":"verdict","session":"s","head_sha":"h","approved":true,"confidence":0.95,"major":1}"#
        );
        assert_eq!(parse_record(&r.to_line()), Some(r));
        let old = r#"{"bosun":"verdict","session":"s","head_sha":"h","approved":true}"#;
        assert!(matches!(
            parse_record(old),
            Some(Record::Verdict {
                confidence: None,
                ..
            })
        ));
    }

    #[test]
    fn lesson_seen_round_trips() {
        let r = Record::LessonSeen {
            session: "s".into(),
            signature: "00ff00ff00ff00ff".into(),
            issue: Some(7),
        };
        assert_eq!(
            r.to_line(),
            r#"{"bosun":"lesson-seen","session":"s","signature":"00ff00ff00ff00ff","issue":7}"#
        );
        assert_eq!(parse_record(&r.to_line()), Some(r));
    }

    #[test]
    fn approve_record_has_no_session() {
        let r = Record::Approve {
            prs: vec![PrHead {
                pr: 4,
                head_sha: "abc".into(),
            }],
            main_open: vec![],
            at: "2026-10-22T10:00:00Z".into(),
        };
        assert_eq!(
            r.to_line(),
            r#"{"bosun":"approve","prs":[{"pr":4,"head_sha":"abc"}],"at":"2026-10-22T10:00:00Z"}"#
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
