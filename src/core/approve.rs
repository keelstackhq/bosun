//! `bosun approve`: which open PRs wait for an owner and are ready for one (spec section 8).
//! Pure: the command reads the statuses and check runs and passes them in.

/// The start of the `risk-check` description while a high-risk PR waits for an owner.
pub const WAITING_PREFIX: &str = "high risk, owner approval needed";

/// A commit status: its context, state and description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stat {
    pub context: String,
    pub state: String,
    pub description: String,
}

/// A check run: `at` is `completed_at`, else `started_at` (empty when GitHub sent neither).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub id: u64,
    pub name: String,
    pub conclusion: Option<String>,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    /// Waits for an owner and both other gates are green; the risk rule ids.
    Ready(Vec<String>),
    /// Waits for an owner, but `why` is not green.
    Waiting(&'static str),
    /// The risk-check does not wait for an owner.
    NotWaiting,
}

/// The newest status of `context` (`statuses` is the combined status: newest per context).
fn status<'a>(statuses: &'a [Stat], context: &str) -> Option<&'a Stat> {
    statuses.iter().find(|s| s.context == context)
}

/// The rule ids of a waiting description: the comma-separated list after the colon.
pub fn risk_ids(description: &str) -> Vec<String> {
    let Some((_, ids)) = description.split_once(':') else {
        return Vec::new();
    };
    ids.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// `ci-ok` is green: the newest check run of that name concluded `success`; with no such run,
/// a `ci-ok` commit status that is `success`.
pub fn ci_green(statuses: &[Stat], runs: &[Run]) -> bool {
    let newest = runs
        .iter()
        .filter(|r| r.name == "ci-ok")
        .max_by(|a, b| (&a.at, a.id).cmp(&(&b.at, b.id)));
    match newest {
        Some(r) => r.conclusion.as_deref() == Some("success"),
        None => status(statuses, "ci-ok").is_some_and(|s| s.state == "success"),
    }
}

pub fn readiness(statuses: &[Stat], runs: &[Run]) -> Readiness {
    let waiting = status(statuses, "risk-check")
        .filter(|s| s.state == "failure" && s.description.starts_with(WAITING_PREFIX));
    let Some(risk) = waiting else {
        return Readiness::NotWaiting;
    };
    if !status(statuses, "review-verdict").is_some_and(|s| s.state == "success") {
        return Readiness::Waiting("review-verdict not green");
    }
    if !ci_green(statuses, runs) {
        return Readiness::Waiting("ci-ok not green");
    }
    Readiness::Ready(risk_ids(&risk.description))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(context: &str, state: &str, description: &str) -> Stat {
        Stat {
            context: context.into(),
            state: state.into(),
            description: description.into(),
        }
    }

    fn run(id: u64, conclusion: Option<&str>, at: &str) -> Run {
        Run {
            id,
            name: "ci-ok".into(),
            conclusion: conclusion.map(String::from),
            at: at.into(),
        }
    }

    fn base() -> Vec<Stat> {
        vec![
            st(
                "risk-check",
                "failure",
                "high risk, owner approval needed: R-gov, R-ci",
            ),
            st("review-verdict", "success", "approved"),
        ]
    }

    #[test]
    fn risk_ids_come_after_the_colon() {
        assert_eq!(
            risk_ids("high risk, owner approval needed: R-gov, R-ci"),
            ["R-gov", "R-ci"]
        );
        assert!(risk_ids("high risk, owner approval needed").is_empty());
    }

    #[test]
    fn ready_needs_waiting_verdict_and_ci() {
        let runs = [run(1, Some("success"), "2026-10-22T10:00:00Z")];
        assert_eq!(
            readiness(&base(), &runs),
            Readiness::Ready(vec!["R-gov".into(), "R-ci".into()])
        );
    }

    #[test]
    fn newest_status_and_run_decide() {
        let mut s = base();
        s[1].state = "failure".into();
        let runs = [run(1, Some("success"), "t1")];
        assert_eq!(
            readiness(&s, &runs),
            Readiness::Waiting("review-verdict not green")
        );
        // A newer failed run beats an older success, in either list order.
        let runs = [run(2, Some("failure"), "t2"), run(1, Some("success"), "t1")];
        assert_eq!(
            readiness(&base(), &runs),
            Readiness::Waiting("ci-ok not green")
        );
        // Same time: the higher id is newer. An unfinished newest run is not green.
        let runs = [run(1, Some("success"), "t1"), run(2, None, "t1")];
        assert!(!ci_green(&[], &runs));
    }

    #[test]
    fn ci_status_counts_without_a_check_run() {
        let mut s = base();
        s.push(st("ci-ok", "success", ""));
        assert!(matches!(readiness(&s, &[]), Readiness::Ready(_)));
        let other = Run {
            name: "lint".into(),
            ..run(1, Some("success"), "t")
        };
        assert!(matches!(readiness(&s, &[other]), Readiness::Ready(_)));
        assert_eq!(
            readiness(&base(), &[]),
            Readiness::Waiting("ci-ok not green")
        );
    }

    #[test]
    fn other_risk_states_do_not_wait() {
        assert_eq!(readiness(&[], &[]), Readiness::NotWaiting);
        let s = [st("risk-check", "success", "normal risk")];
        assert_eq!(readiness(&s, &[]), Readiness::NotWaiting);
        let s = [st(
            "risk-check",
            "success",
            "high risk (R-gov), approved by ada",
        )];
        assert_eq!(readiness(&s, &[]), Readiness::NotWaiting);
    }
}
