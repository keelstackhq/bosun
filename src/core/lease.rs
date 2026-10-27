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

/// The Slack thread ts (`bosun spawn --slack-thread`) of the live lease `session` holds on
/// `issue`, with the id of the comment that carries it; `None` when it holds none or that lease
/// came from no thread. Never another session's lease and never an expired one. The caller
/// must check that the comment's author has write access before it trusts the ts.
pub fn thread_of(
    issue: u64,
    session: &str,
    now: Timestamp,
    notes: &[Note],
) -> Option<(String, u64)> {
    let lease = leases_of(issue, notes)
        .into_iter()
        .find(|l| l.session == session && l.is_live(now))?;
    let note = notes.iter().find(|n| n.id == lease.comment_id)?;
    match parse_record(&note.body) {
        Some(Record::Lease { thread_ts, .. }) => thread_ts.map(|t| (t, note.id)),
        _ => None,
    }
}

/// The newest lease per session on one issue, ended leases removed. Expired ones are kept
/// (reap needs them); filter with `is_live`.
pub fn leases_of(issue: u64, notes: &[Note]) -> Vec<Lease> {
    timed_leases_of(issue, notes)
        .into_iter()
        .map(|(l, _)| l)
        .collect()
}

/// The lease records of a session on one issue since its last lease-end:
/// (creation time, comment id, touch).
type History = Vec<(Timestamp, u64, Vec<String>)>;

/// `leases_of` with the history of each lease (for `race_winner`).
fn timed_leases_of(issue: u64, notes: &[Note]) -> Vec<(Lease, History)> {
    let mut by_session: BTreeMap<String, (Lease, History)> = BTreeMap::new();
    for note in notes {
        match parse_record(&note.body) {
            Some(Record::Lease {
                session,
                origin,
                expires,
                touch,
                ..
            }) => {
                // An unparseable expiry counts as already expired.
                let expires = parse_rfc3339(&expires).unwrap_or(Timestamp(0));
                let mut hist = by_session.remove(&session).map(|e| e.1).unwrap_or_default();
                hist.push((note.created_at, note.id, touch.clone()));
                by_session.insert(
                    session.clone(),
                    (
                        Lease {
                            issue,
                            session,
                            origin,
                            expires,
                            touch,
                            comment_id: note.id,
                        },
                        hist,
                    ),
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

/// Overlaps of `touch` with live leases. `claiming` is the issue being claimed: a lease of
/// `me` on that issue is the same claim and never conflicts, but every other lease conflicts,
/// also one of the same session, because wave agents share their orchestrator's session id
/// (spec §4). With `None` (the edit hook) the session rule applies: leases of `me` and of the
/// orchestrator session `me` runs in (`S` for `S/task`) do not conflict. A lease with a bad
/// glob is treated as covering everything, so a broken record never lets work through.
pub fn conflicts(
    touch: &[Glob],
    leases: &[Lease],
    me: &str,
    claiming: Option<u64>,
) -> Vec<Conflict> {
    leases
        .iter()
        .filter(|l| match claiming {
            Some(issue) => !(l.session == me && l.issue == issue),
            None => l.session != me && l.session != orchestrator_of(me),
        })
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

/// The orchestrator session of a per-task id `S/task` (`BOSUN_SESSION=<session>/<task>`);
/// any other id is its own orchestrator.
pub fn orchestrator_of(session: &str) -> &str {
    session.split_once('/').map_or(session, |(s, _)| s)
}

/// The lease that beat a new lease in a claim race: a live lease, other than the new one, on
/// the same issue or with overlapping touch paths, that wrote first on a contested glob pair.
/// For each pair of globs `g` (of the new lease's current touch) and `h` (of the rival's) that
/// overlap, a lease counts from its session's earliest record since its last lease-end that has a
/// glob overlapping the other's glob; the rival wins the pair when that time is earlier. Both
/// sides compute the same two times per pair, so they can never both win. So a lease that grew
/// keeps its priority on the globs it had and races at the time of the grow for the ones it
/// added. Rivals on the same issue with no overlapping pair are compared by first record.
/// `issues` is the freshly read comments of the issues that may hold one.
pub fn race_winner(
    issue: u64,
    comment_id: u64,
    touch: &[Glob],
    issues: &[(u64, Vec<Note>)],
    now: Timestamp,
) -> Option<Lease> {
    let all = || {
        issues
            .iter()
            .flat_map(|(n, notes)| timed_leases_of(*n, notes))
    };
    let (mine, my_hist) = all().find(|(l, _)| l.issue == issue && l.comment_id == comment_id)?;
    let my_globs = globs_of(&mine.touch);
    all()
        .filter(|(l, _)| l.is_live(now) && l.comment_id != comment_id)
        .filter(|(l, _)| {
            l.issue == issue || glob::first_overlap(touch, &globs_of(&l.touch)).is_some()
        })
        .filter_map(|(l, hist)| {
            let their_globs = globs_of(&l.touch);
            let pairs: Vec<_> = my_globs
                .iter()
                .flat_map(|g| their_globs.iter().map(move |h| (g, h)))
                .filter(|(g, h)| g.overlaps(h))
                .collect();
            let first = |h: &History| h.first().map(|(at, id, _)| (*at, *id));
            let times = if pairs.is_empty() {
                vec![(first(&hist), first(&my_hist))]
            } else {
                pairs
                    .into_iter()
                    .map(|(g, h)| (earliest(&hist, g), earliest(&my_hist, h)))
                    .collect()
            };
            times
                .into_iter()
                .filter_map(|(theirs, mine)| theirs.filter(|t| mine.is_none_or(|m| *t < m)))
                .min()
                .map(|t| (t, l))
        })
        .min_by_key(|(t, _)| *t)
        .map(|(_, l)| l)
}

fn globs_of(touch: &[String]) -> Vec<Glob> {
    glob::parse_all(touch).unwrap_or_else(|_| vec![everything()])
}

/// (time, id) of the earliest record in `hist` with a glob that overlaps `other`.
fn earliest(hist: &History, other: &Glob) -> Option<(Timestamp, u64)> {
    hist.iter()
        .find(|(_, _, t)| globs_of(t).iter().any(|g| g.overlaps(other)))
        .map(|(at, id, _)| (*at, *id))
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
        Note::new(id, format!("bosun record\n{}", r.to_line()))
    }

    fn lease(session: &str, expires: &str, touch: &[&str]) -> Record {
        Record::Lease {
            session: session.into(),
            origin: "local".into(),
            expires: expires.into(),
            touch: touch.iter().map(|s| s.to_string()).collect(),
            thread_ts: None,
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

    fn held(issue: u64, session: &str, touch: &[&str]) -> Lease {
        Lease {
            issue,
            session: session.into(),
            origin: "local".into(),
            expires: Timestamp(i64::MAX),
            touch: touch.iter().map(|s| s.to_string()).collect(),
            comment_id: issue,
        }
    }

    #[test]
    fn the_same_session_conflicts_on_another_issue_but_not_on_its_own() {
        let leases = [held(7, "S", &["src/**"])];
        let touch = glob::parse_all(&["src/a.rs".into()]).unwrap();
        assert_eq!(conflicts(&touch, &leases, "S", Some(8)).len(), 1);
        assert!(conflicts(&touch, &leases, "S", Some(7)).is_empty());
    }

    #[test]
    fn per_task_session_ids_conflict_with_each_other_only() {
        let leases = [held(7, "S/x", &["src/**"])];
        let touch = glob::parse_all(&["src/a.rs".into()]).unwrap();
        assert_eq!(conflicts(&touch, &leases, "S/y", Some(8)).len(), 1);
        assert!(conflicts(&touch, &leases, "S/x", Some(7)).is_empty());
        // The edit hook: S/x conflicts with S/y, and S/y may edit what S leased.
        assert_eq!(conflicts(&touch, &leases, "S/y", None).len(), 1);
        assert!(conflicts(&touch, &leases, "S/x", None).is_empty());
        let orchestrator = [held(7, "S", &["src/**"])];
        assert!(conflicts(&touch, &orchestrator, "S/y", None).is_empty());
        assert_eq!(conflicts(&touch, &orchestrator, "T/y", None).len(), 1);
    }

    #[test]
    fn the_earlier_comment_wins_a_race() {
        let at = |id, secs| {
            let mut n = note(id, lease("a", "2026-10-22T10:30:00Z", &["src/**"]));
            n.created_at = Timestamp(secs);
            n
        };
        let mut rival = note(9, lease("b", "2026-10-22T10:30:00Z", &["src/x/**"]));
        rival.created_at = Timestamp(100);
        let touch = glob::parse_all(&["src/**".into()]).unwrap();
        let issues = vec![(1, vec![at(5, 100)]), (2, vec![rival.clone()])];
        // Same time: the lower comment id wins, so the rival (9) loses to 5.
        assert_eq!(race_winner(1, 5, &touch, &issues, NOW), None);
        let w = race_winner(2, 9, &touch, &issues, NOW).unwrap();
        assert_eq!((w.issue, w.session.as_str()), (1, "a"));
        // An earlier time beats a lower id; a disjoint lease never wins.
        rival.created_at = Timestamp(99);
        let issues = vec![(1, vec![at(5, 100)]), (2, vec![rival])];
        assert_eq!(race_winner(1, 5, &touch, &issues, NOW).unwrap().issue, 2);
        let docs = glob::parse_all(&["docs/**".into()]).unwrap();
        assert_eq!(race_winner(1, 5, &docs, &issues, NOW), None);
    }

    #[test]
    fn a_grown_lease_races_pairwise_from_both_sides() {
        let at = |id, secs, r| {
            let mut n = note(id, r);
            n.created_at = Timestamp(secs);
            n
        };
        let g = |p: &str| glob::parse_all(&[p.to_string()]).unwrap();
        let exp = "2026-10-22T10:30:00Z";
        // X: #1 src/a/** at 100 (comment 1), grown by src/b/** at 200 (comment 3).
        // Y: #2 src/b/x.rs at 150. Z: #3 src/a/x.rs at 150.
        let r1 = at(1, 100, lease("x", exp, &["src/a/**"]));
        let r3 = at(3, 200, lease("x", exp, &["src/a/**", "src/b/**"]));
        let y = at(2, 150, lease("y", exp, &["src/b/x.rs"]));
        let z = at(4, 150, lease("z", exp, &["src/a/x.rs"]));
        let full = vec![
            (1, vec![r1.clone(), r3.clone()]),
            (2, vec![y.clone()]),
            (3, vec![z.clone()]),
        ];
        // (a) the old globs keep their priority against a rival that claimed between.
        assert_eq!(race_winner(1, 3, &g("src/a/**"), &full, NOW), None);
        assert_eq!(
            race_winner(3, 4, &g("src/a/x.rs"), &full, NOW)
                .unwrap()
                .session,
            "x"
        );
        // (b) the added globs race at grow time and lose to the earlier rival.
        assert_eq!(
            race_winner(1, 3, &g("src/b/**"), &full, NOW)
                .unwrap()
                .session,
            "y"
        );
        // (c) from Y's side the same notes give no second winner.
        assert_eq!(race_winner(2, 2, &g("src/b/x.rs"), &full, NOW), None);
        // (d) Y's stale snapshot (without the grow) agrees: Y wins, X loses.
        let stale = vec![(1, vec![r1.clone()]), (2, vec![y.clone()])];
        assert_eq!(race_winner(2, 2, &g("src/b/x.rs"), &stale, NOW), None);
        // (e) a lease-end resets the history: X's re-claim at 400 loses to Z.
        let again = at(5, 400, lease("x", exp, &["src/a/**"]));
        let reset = vec![(1, vec![r1, at(6, 300, end("x")), again]), (3, vec![z])];
        assert_eq!(
            race_winner(1, 5, &g("src/a/**"), &reset, NOW)
                .unwrap()
                .session,
            "z"
        );
    }

    #[test]
    fn a_grow_loses_only_on_the_contested_glob() {
        let at = |id, secs, r| {
            let mut n = note(id, r);
            n.created_at = Timestamp(secs);
            n
        };
        let g = |ps: &[&str]| {
            glob::parse_all(&ps.iter().map(|p| p.to_string()).collect::<Vec<_>>()).unwrap()
        };
        let exp = "2026-10-22T10:30:00Z";
        // X r1@100 {a/**}; Y s1@150 {b/**}, grown s2@250 {b/**, a/x.rs}; X grows r2@260.
        let both = ["src/a/**", "src/b/**"];
        let issues = vec![
            (
                1,
                vec![
                    at(1, 100, lease("x", exp, &["src/a/**"])),
                    at(4, 260, lease("x", exp, &both)),
                ],
            ),
            (
                2,
                vec![
                    at(2, 150, lease("y", exp, &["src/b/**"])),
                    at(3, 250, lease("y", exp, &["src/b/**", "src/a/x.rs"])),
                ],
            ),
        ];
        // X's grow loses on src/b, to Y, whose lease keeps src/b.
        let w = race_winner(1, 4, &g(&both), &issues, NOW).unwrap();
        assert_eq!(w.session, "y");
        // Y's grow (comment 3) loses on src/a/x.rs, which X has held since 100. Both grows
        // lose; that only costs a retry, they can never both win.
        let w = race_winner(2, 3, &g(&["src/a/x.rs"]), &issues, NOW).unwrap();
        assert_eq!(w.session, "x");
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
        let got = conflicts(&touch, &live, "me", None);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].issue, 2);
        assert_eq!(
            got[0].describe(),
            "`src/core/lease.rs` overlaps `src/core/*.rs` leased on #2 by other until 2026-10-22T10:30:00Z"
        );
        let elsewhere = glob::parse_all(&["docs/**".into()]).unwrap();
        assert!(conflicts(&elsewhere, &live, "me", None).is_empty());
    }

    #[test]
    fn a_lease_with_a_bad_glob_blocks_everything() {
        let issues = vec![(
            4,
            vec![note(1, lease("o", "2026-10-22T10:30:00Z", &["../x"]))],
        )];
        let touch = glob::parse_all(&["docs/a.md".into()]).unwrap();
        assert_eq!(
            conflicts(&touch, &live_leases(&issues, NOW), "me", None).len(),
            1
        );
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
