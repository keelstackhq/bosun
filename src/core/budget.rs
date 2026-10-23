//! The spend guard of spec §9: the level for today's spend, and which sessions to stop.

use super::lease::{Member, orchestrator_of};
use crate::time::Timestamp;

/// How much of the daily budget is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Below 80 %.
    Normal,
    /// 80 % or more: no new work.
    Guard,
    /// 100 % or more: stop the live sessions.
    Stop,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Normal => "normal",
            Level::Guard => "guard",
            Level::Stop => "stop",
        }
    }
}

/// Share of the daily budget at which new work stops (spec §9).
pub const BUDGET_STOP: f64 = 0.8;

/// The one 80 % rule: at or over `BUDGET_STOP` of a daily budget, no new work starts. A budget
/// of 0 or less is off.
pub fn no_new_work(spent: f64, daily: f64) -> bool {
    daily > 0.0 && spent >= BUDGET_STOP * daily
}

/// The level of `spent` against `daily`. A budget of 0 or less is off: always normal.
pub fn level(spent: f64, daily: f64) -> Level {
    if daily <= 0.0 {
        Level::Normal
    } else if spent >= daily {
        Level::Stop
    } else if no_new_work(spent, daily) {
        Level::Guard
    } else {
        Level::Normal
    }
}

/// The sessions to stop: only at `Stop`, the remote sessions of the live members, sorted and
/// without repeats. A wave agent `S/task` stands for its session `S`; the caller's own session
/// is never stopped; ids that are not remote sessions (`local-<host>`) cannot be interrupted.
pub fn to_stop(level: Level, members: &[Member], caller: &str, now: Timestamp) -> Vec<String> {
    if level != Level::Stop {
        return Vec::new();
    }
    let me = orchestrator_of(caller);
    let mut ids: Vec<String> = members
        .iter()
        .filter(|m| m.expires > now)
        .map(|m| orchestrator_of(&m.session))
        .filter(|s| *s != me && s.starts_with("session_"))
        .map(str::to_string)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(session: &str, expires: i64) -> Member {
        Member {
            session: session.into(),
            origin: "spawned".into(),
            area: None,
            parent: None,
            depth: 1,
            expires: Timestamp(expires),
            comment_id: 1,
        }
    }

    #[test]
    fn levels_follow_the_thresholds() {
        assert_eq!(level(0.0, 150.0), Level::Normal);
        assert_eq!(level(119.9, 150.0), Level::Normal);
        assert_eq!(level(120.0, 150.0), Level::Guard);
        assert_eq!(level(149.9, 150.0), Level::Guard);
        assert_eq!(level(150.0, 150.0), Level::Stop);
        assert_eq!(level(900.0, 0.0), Level::Normal);
    }

    #[test]
    fn no_new_work_agrees_with_level() {
        for (spent, daily) in [
            (0.0, 150.0),
            (119.9, 150.0),
            (120.0, 150.0),
            (150.0, 150.0),
            (900.0, 0.0),
            (5.0, -1.0),
        ] {
            assert_eq!(
                no_new_work(spent, daily),
                level(spent, daily) != Level::Normal
            );
        }
        assert!(!no_new_work(79.9, 100.0));
        assert!(no_new_work(80.0, 100.0));
    }

    #[test]
    fn only_stop_names_live_members_but_the_caller() {
        let ms = [
            member("session_b", 200),
            member("session_a", 200),
            member("session_old", 50),
            member("session_me", 200),
        ];
        let now = Timestamp(100);
        assert_eq!(
            to_stop(Level::Stop, &ms, "session_me", now),
            ["session_a", "session_b"]
        );
        assert!(to_stop(Level::Guard, &ms, "session_me", now).is_empty());
        assert!(to_stop(Level::Normal, &ms, "session_me", now).is_empty());
    }

    #[test]
    fn wave_agents_fold_into_their_session_and_local_ids_are_skipped() {
        let ms = [
            member("session_a", 200),
            member("session_a/M4-T1", 200),
            member("session_b/M4-T2", 200),
            member("session_me/M4-T3", 200),
            member("local-host", 200),
        ];
        let now = Timestamp(100);
        // The caller is a wave agent of session_me: its orchestrator is not stopped either.
        assert_eq!(
            to_stop(Level::Stop, &ms, "session_me/M4-T4", now),
            ["session_a", "session_b"]
        );
    }
}
