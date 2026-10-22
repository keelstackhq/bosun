//! Leases and registry entries from comment records (spec §4). A lease is live when it is the
//! newest lease of its session on the issue, no later `lease-end` of that session follows,
//! and it has not expired. Heartbeats edit the record in place, so the body is current.

use std::collections::BTreeMap;

use crate::core::glob::{self, Glob};
use crate::core::records::{Note, Record, parse_record};
use crate::time::{Timestamp, parse_rfc3339};

/// Lease time to live (spec §4): 30 minutes.
pub const TTL_SECS: i64 = 1800;

#[derive(Debug, Clone, PartialEq)]
pub struct Lease {
    pub issue: u64,
    pub session: String,
    pub origin: String,
    pub expires: Timestamp,
    pub touch: Vec<String>,
    pub comment_id: u64,
}

impl Lease {
    pub fn is_live(&self, now: Timestamp) -> bool {
        self.expires > now
    }
}

/// The newest lease per session on one issue, ended leases removed. Expired ones are kept
/// (reap needs them); filter with `is_live`.
pub fn leases_of(issue: u64, notes: &[Note]) -> Vec<Lease> {
    let mut by_session: BTreeMap<String, Lease> = BTreeMap::new();
    for note in notes {
        match parse_record(&note.body) {
            Some(Record::Lease {
                session,
                origin,
                expires,
                touch,
            }) => {
                // An unparseable expiry counts as already expired.
                let expires = parse_rfc3339(&expires).unwrap_or(Timestamp(0));
                by_session.insert(
                    session.clone(),
                    Lease {
                        issue,
                        session,
                        origin,
                        expires,
                        touch,
                        comment_id: note.id,
                    },
                );
            }
            Some(Record::LeaseEnd { session, .. }) => {
                by_session.remove(&session);
            }
            _ => {}
        }
    }
    by_session.into_values().collect()
}

/// Live leases across issues.
pub fn live_leases(issues: &[(u64, Vec<Note>)], now: Timestamp) -> Vec<Lease> {
    issues
        .iter()
        .flat_map(|(n, notes)| leases_of(*n, notes))
        .filter(|l| l.is_live(now))
        .collect()
}

/// A lease of another session whose touch paths overlap.
#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    pub issue: u64,
    pub session: String,
    pub mine: String,
    pub theirs: String,
    pub expires: Timestamp,
}

impl Conflict {
    pub fn describe(&self) -> String {
        format!(
            "`{}` overlaps `{}` leased on #{} by {} until {}",
            self.mine, self.theirs, self.issue, self.session, self.expires
        )
    }
}

/// Overlaps of `touch` with the live leases of other sessions. A lease with a bad glob is
/// treated as covering everything, so a broken record never lets work through.
pub fn conflicts(touch: &[Glob], leases: &[Lease], me: &str) -> Vec<Conflict> {
    leases
        .iter()
        .filter(|l| l.session != me)
        .filter_map(|l| {
            let theirs = glob::parse_all(&l.touch).unwrap_or_else(|_| vec![everything()]);
            glob::first_overlap(touch, &theirs).map(|(mine, theirs)| Conflict {
                issue: l.issue,
                session: l.session.clone(),
                mine,
                theirs,
                expires: l.expires,
            })
        })
        .collect()
}

fn everything() -> Glob {
    Glob::parse("**").expect("valid glob")
}

/// A registry entry.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub session: String,
    pub origin: String,
    pub area: Option<String>,
    pub parent: Option<String>,
    pub depth: u32,
    pub expires: Timestamp,
    pub comment_id: u64,
}

/// The newest `join` per session on the registry issue, minus sessions that left after it.
/// Expired members are kept (reap needs them); filter on `expires`.
pub fn members(notes: &[Note]) -> Vec<Member> {
    let mut by_session: BTreeMap<String, Member> = BTreeMap::new();
    for note in notes {
        match parse_record(&note.body) {
            Some(Record::Join {
                session,
                origin,
                area,
                parent,
                depth,
                expires,
            }) => {
                let expires = parse_rfc3339(&expires).unwrap_or(Timestamp(0));
                by_session.insert(
                    session.clone(),
                    Member {
                        session,
                        origin,
                        area,
                        parent,
                        depth,
                        expires,
                        comment_id: note.id,
                    },
                );
            }
            Some(Record::Leave { session, .. }) => {
                by_session.remove(&session);
            }
            _ => {}
        }
    }
    by_session.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = Timestamp(1_792_663_200); // 2026-10-22T10:00:00Z

    fn note(id: u64, r: Record) -> Note {
        Note {
            id,
            body: format!("bosun record\n{}", r.to_line()),
        }
    }

    fn lease(session: &str, expires: &str, touch: &[&str]) -> Record {
        Record::Lease {
            session: session.into(),
            origin: "local".into(),
            expires: expires.into(),
            touch: touch.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn end(session: &str) -> Record {
        Record::LeaseEnd {
            session: session.into(),
            reason: "released".into(),
        }
    }

    #[test]
    fn now_constant_is_the_test_clock() {
        assert_eq!(NOW.to_string(), "2026-10-22T10:00:00Z");
    }

    #[test]
    fn newest_lease_per_session_wins_and_ends_remove() {
        let notes = vec![
            note(1, lease("a", "2026-10-22T10:10:00Z", &["x/**"])),
            note(2, lease("a", "2026-10-22T10:20:00Z", &["y/**"])),
            note(3, lease("b", "2026-10-22T10:20:00Z", &["z/**"])),
            note(4, end("b")),
        ];
        let got = leases_of(7, &notes);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].session, "a");
        assert_eq!(got[0].touch, vec!["y/**"]);
        assert_eq!(got[0].comment_id, 2);
    }

    #[test]
    fn expired_and_unparseable_leases_are_not_live() {
        let issues = vec![
            (1, vec![note(1, lease("a", "2026-10-22T09:59:59Z", &["x"]))]),
            (2, vec![note(2, lease("b", "garbage", &["x"]))]),
            (3, vec![note(3, lease("c", "2026-10-22T10:00:01Z", &["x"]))]),
        ];
        let live = live_leases(&issues, NOW);
        assert_eq!(live.iter().map(|l| l.issue).collect::<Vec<_>>(), vec![3]);
    }

    #[test]
    fn conflicts_skip_my_own_leases() {
        let issues = vec![
            (
                1,
                vec![note(1, lease("me", "2026-10-22T10:30:00Z", &["src/**"]))],
            ),
            (
                2,
                vec![note(
                    2,
                    lease("other", "2026-10-22T10:30:00Z", &["src/core/*.rs"]),
                )],
            ),
        ];
        let live = live_leases(&issues, NOW);
        let touch = glob::parse_all(&["src/core/lease.rs".into()]).unwrap();
        let got = conflicts(&touch, &live, "me");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].issue, 2);
        assert_eq!(
            got[0].describe(),
            "`src/core/lease.rs` overlaps `src/core/*.rs` leased on #2 by other until 2026-10-22T10:30:00Z"
        );
        let elsewhere = glob::parse_all(&["docs/**".into()]).unwrap();
        assert!(conflicts(&elsewhere, &live, "me").is_empty());
    }

    #[test]
    fn a_lease_with_a_bad_glob_blocks_everything() {
        let issues = vec![(
            4,
            vec![note(1, lease("o", "2026-10-22T10:30:00Z", &["../x"]))],
        )];
        let touch = glob::parse_all(&["docs/a.md".into()]).unwrap();
        assert_eq!(conflicts(&touch, &live_leases(&issues, NOW), "me").len(), 1);
    }

    #[test]
    fn members_follow_join_and_leave() {
        let join = |s: &str, area: Option<&str>| Record::Join {
            session: s.into(),
            origin: "spawned".into(),
            area: area.map(str::to_string),
            parent: Some("root".into()),
            depth: 2,
            expires: "2026-10-22T10:30:00Z".into(),
        };
        let notes = vec![
            note(1, join("a", Some("lifecycle"))),
            note(2, join("b", None)),
            note(
                3,
                Record::Leave {
                    session: "b".into(),
                    reason: "done".into(),
                },
            ),
        ];
        let got = members(&notes);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].area.as_deref(), Some("lifecycle"));
        assert_eq!(got[0].depth, 2);
    }
}
