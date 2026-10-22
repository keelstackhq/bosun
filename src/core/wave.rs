//! Wave selection (spec §5): the next tasks of an epic that can run in parallel.

use crate::core::glob::{self, Glob};
use crate::core::lease::{self, Lease};
use crate::core::state::State;

/// A task issue as wave planning sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskInfo {
    pub issue: u64,
    /// The plan id, for example `M1/T2`.
    pub plan_id: String,
    pub state: Option<State>,
    /// Plan ids this task waits for.
    pub depends: Vec<String>,
    pub touch: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Wave {
    /// Issues to start now, in issue order.
    pub run: Vec<u64>,
    /// Issues not started, with the reason.
    pub waiting: Vec<(u64, String)>,
}

/// Pick ready tasks whose dependencies are merged, whose touch paths overlap no live lease
/// (of any issue but their own: wave agents share the orchestrator's session id) and no task
/// already picked or in flight, up to `max_parallel`. A task of `tasks` that is active (spec
/// §4) and holds a live lease is in flight: it counts against the cap and its touch paths
/// block overlapping picks. `merged` lists plan ids whose tasks merged (tasks of other epics
/// included).
pub fn plan(
    tasks: &[TaskInfo],
    merged: &[String],
    leases: &[Lease],
    me: &str,
    max_parallel: usize,
) -> Wave {
    let mut sorted: Vec<&TaskInfo> = tasks.iter().collect();
    sorted.sort_by_key(|t| t.issue);
    let mut wave = Wave::default();
    let mut picked: Vec<(u64, Vec<Glob>)> = Vec::new();
    let mut in_flight: Vec<(u64, Vec<Glob>)> = Vec::new();
    for t in tasks {
        if t.state.is_some_and(State::is_active) && leases.iter().any(|l| l.issue == t.issue) {
            // A bad glob in a task is already caught by its lease; skip it here.
            in_flight.push((t.issue, glob::parse_all(&t.touch).unwrap_or_default()));
        }
    }
    for t in sorted {
        let wait = |why: String| (t.issue, why);
        if t.state != Some(State::Ready) {
            let st = t.state.map_or("no state".to_string(), |s| s.to_string());
            wave.waiting.push(wait(format!("state is {st}")));
            continue;
        }
        let open: Vec<&String> = t.depends.iter().filter(|d| !merged.contains(d)).collect();
        if !open.is_empty() {
            let list: Vec<&str> = open.iter().map(|s| s.as_str()).collect();
            wave.waiting
                .push(wait(format!("waits for {}", list.join(", "))));
            continue;
        }
        let touch = match glob::parse_all(&t.touch) {
            Ok(g) if !g.is_empty() => g,
            Ok(_) => {
                wave.waiting.push(wait("has no touch paths".into()));
                continue;
            }
            Err(e) => {
                wave.waiting.push(wait(e));
                continue;
            }
        };
        if let Some(c) = lease::conflicts(&touch, leases, me, Some(t.issue)).first() {
            wave.waiting.push(wait(c.describe()));
            continue;
        }
        if let Some((other, (a, b))) = picked
            .iter()
            .find_map(|(n, g)| glob::first_overlap(&touch, g).map(|p| (*n, p)))
        {
            wave.waiting.push(wait(format!(
                "`{a}` overlaps `{b}` of #{other} in this wave"
            )));
            continue;
        }
        if let Some((other, (a, b))) = in_flight
            .iter()
            .find_map(|(n, g)| glob::first_overlap(&touch, g).map(|p| (*n, p)))
        {
            wave.waiting
                .push(wait(format!("`{a}` overlaps `{b}` of #{other} in flight")));
            continue;
        }
        if in_flight.len() + picked.len() >= max_parallel {
            wave.waiting
                .push(wait(format!("waves.max_parallel is {max_parallel}")));
            continue;
        }
        wave.run.push(t.issue);
        picked.push((t.issue, touch));
    }
    wave
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Timestamp;

    fn task(issue: u64, id: &str, state: State, depends: &[&str], touch: &[&str]) -> TaskInfo {
        TaskInfo {
            issue,
            plan_id: id.into(),
            state: Some(state),
            depends: depends.iter().map(|s| s.to_string()).collect(),
            touch: touch.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn dependency_chain_runs_only_the_head() {
        let tasks = [
            task(3, "M1/T3", State::Ready, &["M1/T2"], &["c/**"]),
            task(1, "M1/T1", State::Ready, &[], &["a/**"]),
            task(2, "M1/T2", State::Ready, &["M1/T1"], &["b/**"]),
        ];
        let w = plan(&tasks, &[], &[], "me", 6);
        assert_eq!(w.run, vec![1]);
        assert_eq!(w.waiting[0], (2, "waits for M1/T1".to_string()));
        let w = plan(&tasks, &["M1/T1".into()], &[], "me", 6);
        assert_eq!(w.run, vec![1, 2]);
    }

    #[test]
    fn overlapping_pair_runs_one() {
        let tasks = [
            task(1, "M1/T1", State::Ready, &[], &["src/core/**"]),
            task(2, "M1/T2", State::Ready, &[], &["src/core/wave.rs"]),
            task(3, "M1/T3", State::Ready, &[], &["docs/**"]),
        ];
        let w = plan(&tasks, &[], &[], "me", 6);
        assert_eq!(w.run, vec![1, 3]);
        assert!(w.waiting[0].1.contains("of #1 in this wave"));
    }

    #[test]
    fn parallel_cap_and_states() {
        let tasks = [
            task(1, "A/T1", State::Ready, &[], &["a"]),
            task(2, "A/T2", State::Ready, &[], &["b"]),
            task(3, "A/T3", State::Ready, &[], &["c"]),
            task(4, "A/T4", State::Building, &[], &["d"]),
            task(5, "A/T5", State::Ready, &[], &[]),
        ];
        let w = plan(&tasks, &[], &[], "me", 2);
        assert_eq!(w.run, vec![1, 2]);
        let reasons: Vec<&str> = w.waiting.iter().map(|(_, r)| r.as_str()).collect();
        assert_eq!(
            reasons,
            vec![
                "waves.max_parallel is 2",
                "state is building",
                "has no touch paths"
            ]
        );
    }

    #[test]
    fn live_lease_of_another_session_blocks() {
        let lease = Lease {
            issue: 9,
            session: "other".into(),
            origin: "local".into(),
            expires: Timestamp(i64::MAX),
            touch: vec!["a/**".into()],
            comment_id: 1,
        };
        let tasks = [task(1, "A/T1", State::Ready, &[], &["a/x.rs"])];
        assert!(
            plan(&tasks, &[], std::slice::from_ref(&lease), "me", 6)
                .run
                .is_empty()
        );
        // The same session does not excuse a lease on another issue.
        assert!(plan(&tasks, &[], &[lease], "other", 6).run.is_empty());
    }

    #[test]
    fn in_flight_tasks_count_against_the_cap_and_block_overlap() {
        let lease = |issue: u64| Lease {
            issue,
            session: "me".into(),
            origin: "local".into(),
            expires: Timestamp(i64::MAX),
            touch: vec![],
            comment_id: issue,
        };
        let tasks = [
            task(1, "A/T1", State::Building, &[], &["a/**"]),
            task(2, "A/T2", State::Fixing, &[], &["b/**"]),
            // Active without a live lease (expired): not in flight.
            task(3, "A/T3", State::Claimed, &[], &["e/**"]),
            task(4, "A/T4", State::Ready, &[], &["a/x.rs"]),
            task(5, "A/T5", State::Ready, &[], &["c/**"]),
            task(6, "A/T6", State::Ready, &[], &["d/**"]),
        ];
        let leases = [lease(1), lease(2)];
        let w = plan(&tasks, &[], &leases, "me", 3);
        assert_eq!(w.run, vec![5]);
        let reasons: Vec<&str> = w.waiting.iter().map(|(_, r)| r.as_str()).collect();
        assert!(reasons.contains(&"waves.max_parallel is 3"), "{reasons:?}");
        // With a cap of 1 nothing starts while a task is in flight.
        assert!(plan(&tasks, &[], &leases, "me", 2).run.is_empty());
    }
}
