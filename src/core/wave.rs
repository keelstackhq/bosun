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
    /// Single-shape epic: the other issues done in the same PR as `run[0]` (empty for split).
    pub bundle: Vec<u64>,
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

/// Wave of a single-shape epic: all its ready tasks are one PR, led by the lowest issue. Nothing
/// runs while one of its tasks is in flight, while a dependency outside the bundle is open, or
/// while the union of the touch paths conflicts with a lease of another issue.
pub fn plan_single(tasks: &[TaskInfo], merged: &[String], leases: &[Lease], me: &str) -> Wave {
    let mut sorted: Vec<&TaskInfo> = tasks.iter().collect();
    sorted.sort_by_key(|t| t.issue);
    let mut wave = Wave::default();
    let inflight = sorted
        .iter()
        // Any active member is in flight, lease or not: the bundled issues stay `ready` until the
        // lead's PR merges and closes them, so a lapsed lease must not hand them out again.
        .find(|t| t.state.is_some_and(State::is_active))
        .map(|t| t.issue);
    let mut members: Vec<&TaskInfo> = Vec::new();
    for t in sorted {
        if t.state == Some(State::Ready) {
            members.push(t);
        } else {
            let st = t.state.map_or("no state".to_string(), |s| s.to_string());
            wave.waiting.push((t.issue, format!("state is {st}")));
        }
    }
    let hold = |why: Option<String>, members: &[&TaskInfo], wave: &mut Wave| match why {
        Some(why) => {
            wave.waiting
                .extend(members.iter().map(|t| (t.issue, why.clone())));
            true
        }
        None => false,
    };
    if hold(
        inflight.map(|n| format!("bundled into #{n}'s PR")),
        &members,
        &mut wave,
    ) {
        return wave;
    }
    let ids: Vec<&String> = members.iter().map(|t| &t.plan_id).collect();
    let mut open: Vec<&str> = Vec::new();
    for d in members.iter().flat_map(|t| &t.depends) {
        if !merged.contains(d) && !ids.contains(&d) && !open.contains(&d.as_str()) {
            open.push(d);
        }
    }
    let open = (!open.is_empty()).then(|| format!("waits for {}", open.join(", ")));
    if hold(open, &members, &mut wave) {
        return wave;
    }
    let mut touch: Vec<String> = Vec::new();
    for g in members.iter().flat_map(|t| &t.touch) {
        if !touch.contains(g) {
            touch.push(g.clone());
        }
    }
    let globs = match glob::parse_all(&touch) {
        Ok(g) if !g.is_empty() => g,
        Ok(_) => {
            hold(Some("has no touch paths".into()), &members, &mut wave);
            return wave;
        }
        Err(e) => {
            hold(Some(e), &members, &mut wave);
            return wave;
        }
    };
    // Only my own leases on the members are mine; another session's lease on one still conflicts.
    let own = |l: &Lease| l.session == me && members.iter().any(|t| t.issue == l.issue);
    let others: Vec<Lease> = leases.iter().filter(|l| !own(l)).cloned().collect();
    let lead = members.first().map(|t| t.issue);
    let conflict = lease::conflicts(&globs, &others, me, lead)
        .first()
        .map(|c| c.describe());
    if hold(conflict, &members, &mut wave) {
        return wave;
    }
    if let Some((lead, rest)) = members.split_first() {
        wave.run.push(lead.issue);
        wave.bundle = rest.iter().map(|t| t.issue).collect();
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

    fn lease(issue: u64, session: &str, touch: &[&str]) -> Lease {
        Lease {
            issue,
            session: session.into(),
            origin: "local".into(),
            expires: Timestamp(i64::MAX),
            touch: touch.iter().map(|s| s.to_string()).collect(),
            comment_id: issue,
        }
    }

    fn trio() -> Vec<TaskInfo> {
        vec![
            task(7, "M1/T3", State::Ready, &["M1/T2"], &["c/**", "a/**"]),
            task(5, "M1/T1", State::Ready, &[], &["a/**"]),
            task(6, "M1/T2", State::Ready, &["M1/T1"], &["b/**"]),
        ]
    }

    #[test]
    fn single_bundles_ready_tasks_behind_the_lowest_issue() {
        let w = plan_single(&trio(), &[], &[], "me");
        assert_eq!((w.run, w.bundle), (vec![5], vec![6, 7]));
        assert!(w.waiting.is_empty());
    }

    #[test]
    fn single_in_flight_member_runs_nothing() {
        let mut t = trio();
        t.push(task(4, "M1/T0", State::Building, &[], &["z/**"]));
        let w = plan_single(&t, &[], &[lease(4, "me", &["z/**"])], "me");
        assert!(w.run.is_empty() && w.bundle.is_empty());
        assert!(w.waiting.contains(&(5, "bundled into #4's PR".to_string())));
        assert!(w.waiting.contains(&(4, "state is building".to_string())));
        // Active without a live lease (the lead's PR waits on CI): still in flight.
        assert!(plan_single(&t, &[], &[], "me").run.is_empty());
    }

    #[test]
    fn single_respects_another_sessions_lease_on_a_member() {
        let t = trio();
        let w = plan_single(&t, &[], &[lease(5, "other", &["a/**"])], "me");
        assert!(w.run.is_empty(), "{w:?}");
        assert_eq!(
            plan_single(&t, &[], &[lease(5, "me", &["a/**"])], "me").run,
            vec![5]
        );
    }

    #[test]
    fn single_external_dependency_holds_the_whole_bundle() {
        let mut t = trio();
        t[1].depends = vec!["X1/T1".into()];
        let w = plan_single(&t, &[], &[], "me");
        assert!(w.run.is_empty());
        assert_eq!(w.waiting.len(), 3);
        assert!(w.waiting.iter().all(|(_, r)| r == "waits for X1/T1"));
        assert_eq!(plan_single(&t, &["X1/T1".into()], &[], "me").run, vec![5]);
    }

    #[test]
    fn single_waits_on_state_touch_and_foreign_leases() {
        let mut t = trio();
        t.push(task(8, "M1/T4", State::Blocked, &[], &["d"]));
        let w = plan_single(&t, &[], &[], "me");
        assert_eq!(w.run, vec![5]);
        assert_eq!(w.waiting, vec![(8, "state is blocked".to_string())]);
        t[0].touch.clear();
        t[1].touch.clear();
        t[2].touch.clear();
        let w = plan_single(&t, &[], &[], "me");
        assert!(w.run.is_empty());
        assert!(w.waiting.iter().any(|(_, r)| r == "has no touch paths"));
        let t = trio();
        let w = plan_single(&t, &[], &[lease(99, "other", &["b/x.rs"])], "me");
        assert!(w.run.is_empty());
        assert!(
            w.waiting.iter().all(|(_, r)| r.contains("#99")),
            "{:?}",
            w.waiting
        );
    }
}
