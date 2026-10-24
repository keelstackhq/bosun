//! `bosun approve`: which open PRs wait for an owner and are ready for one (spec section 8).
//! Pure: the command reads the statuses and check runs and passes them in.

/// The start of the `risk-check` description while a high-risk PR waits for an owner.
pub const WAITING_PREFIX: &str = "high risk, owner approval needed";
/// The only creator whose `risk-check` and `review-verdict` statuses count: the Actions token.
/// Anyone with write access (an agent) can post a status under any context. Posting as this
/// creator needs a workflow run in the repo, which raises the bar but is not a guarantee: a
/// pushed branch with an `on: push` workflow and `statuses: write` can post as it too.
pub const TRUSTED_CREATOR: &str = "github-actions[bot]";
/// The only app whose `ci-ok` check run counts.
pub const TRUSTED_APP: &str = "github-actions";

/// A commit status: its context, state, description and who posted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stat {
    pub context: String,
    pub state: String,
    pub description: String,
    pub creator: String,
}

/// A check run: `at` is `completed_at`, else `started_at` (empty when GitHub sent neither).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub id: u64,
    pub name: String,
    pub conclusion: Option<String>,
    pub at: String,
    pub app: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Readiness {
    /// Waits for an owner and both other gates are green; the risk rule ids from the status text.
    Ready(Vec<String>),
    /// Waits for an owner, but `why` is not green.
    Waiting(&'static str),
    /// The risk-check does not wait for an owner.
    NotWaiting,
}

/// The newest status of `context` posted by the Actions token (`statuses` is newest first).
fn status<'a>(statuses: &'a [Stat], context: &str) -> Option<&'a Stat> {
    statuses
        .iter()
        .find(|s| s.context == context && s.creator == TRUSTED_CREATOR)
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

/// `ci-ok` is green: the newest check run of that name created by GitHub Actions concluded
/// `success`. A `ci-ok` commit status never counts (anyone with write access can post one).
pub fn ci_green(runs: &[Run]) -> bool {
    runs.iter()
        .filter(|r| r.name == "ci-ok" && r.app == TRUSTED_APP)
        .max_by(|a, b| (&a.at, a.id).cmp(&(&b.at, b.id)))
        .is_some_and(|r| r.conclusion.as_deref() == Some("success"))
}

/// `statuses` newest first.
pub fn readiness(statuses: &[Stat], runs: &[Run]) -> Readiness {
    let waiting = status(statuses, "risk-check")
        .filter(|s| s.state == "failure" && s.description.starts_with(WAITING_PREFIX));
    let Some(risk) = waiting else {
        return Readiness::NotWaiting;
    };
    if !status(statuses, "review-verdict").is_some_and(|s| s.state == "success") {
        return Readiness::Waiting("review-verdict not green");
    }
    if !ci_green(runs) {
        return Readiness::Waiting("ci-ok not green");
    }
    Readiness::Ready(risk_ids(&risk.description))
}

/// `<pr>@<40-hex head sha>` -> (pr, lowercase sha).
pub fn parse_target(arg: &str) -> Option<(u64, String)> {
    let (pr, sha) = arg.split_once('@')?;
    let ok = sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit());
    ok.then_some((pr.parse().ok()?, sha.to_ascii_lowercase()))
}

/// The command that approves exactly this list; `None` when it is empty.
pub fn approve_command(ready: &[(u64, String)]) -> Option<String> {
    if ready.is_empty() {
        return None;
    }
    let args: Vec<String> = ready
        .iter()
        .map(|(pr, sha)| format!("{pr}@{sha}"))
        .collect();
    Some(format!("bosun approve {}", args.join(" ")))
}

/// The owner token only goes to https, or to a loopback host (the tests' fake).
pub fn api_url_ok(url: &str) -> bool {
    let loopback = |host: &str| {
        url.strip_prefix(host)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
    };
    url.starts_with("https://") || loopback("http://127.0.0.1") || loopback("http://localhost")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(context: &str, state: &str, description: &str) -> Stat {
        by(context, state, description, TRUSTED_CREATOR)
    }

    fn by(context: &str, state: &str, description: &str, creator: &str) -> Stat {
        Stat {
            context: context.into(),
            state: state.into(),
            description: description.into(),
            creator: creator.into(),
        }
    }

    fn run(id: u64, conclusion: Option<&str>, at: &str) -> Run {
        Run {
            id,
            name: "ci-ok".into(),
            conclusion: conclusion.map(String::from),
            at: at.into(),
            app: TRUSTED_APP.into(),
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
        assert!(!ci_green(&runs));
        // The first (newest) trusted status of a context decides.
        let s = [
            st("review-verdict", "failure", ""),
            st("review-verdict", "success", ""),
        ];
        assert!(status(&s, "review-verdict").is_some_and(|s| s.state == "failure"));
    }

    #[test]
    fn statuses_from_anyone_else_do_not_count() {
        // An agent's newer statuses neither hide nor fake the Actions ones.
        let mut s = vec![
            by("review-verdict", "failure", "", "octocat"),
            by("risk-check", "success", "normal risk", "octocat"),
        ];
        s.extend(base());
        let runs = [run(1, Some("success"), "t")];
        assert!(matches!(readiness(&s, &runs), Readiness::Ready(_)));
        // Only agent statuses: nothing waits, nothing is ready.
        let forged = [
            by("risk-check", "failure", WAITING_PREFIX, "octocat"),
            by("review-verdict", "success", "", "octocat"),
        ];
        assert_eq!(readiness(&forged, &runs), Readiness::NotWaiting);
        let no_verdict = [base()[0].clone(), forged[1].clone()];
        assert_eq!(
            readiness(&no_verdict, &runs),
            Readiness::Waiting("review-verdict not green")
        );
    }

    #[test]
    fn ci_ok_counts_only_from_actions() {
        let other = Run {
            app: "some-app".into(),
            ..run(1, Some("success"), "t")
        };
        assert!(!ci_green(&[other]));
        assert!(!ci_green(&[]));
        let lint = Run {
            name: "lint".into(),
            ..run(1, Some("success"), "t")
        };
        assert!(!ci_green(&[lint]));
        // A ci-ok commit status is no replacement.
        let mut s = base();
        s.push(st("ci-ok", "success", ""));
        assert_eq!(readiness(&s, &[]), Readiness::Waiting("ci-ok not green"));
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

    #[test]
    fn targets_name_the_head() {
        let sha = "a".repeat(40);
        assert_eq!(parse_target(&format!("12@{sha}")), Some((12, sha.clone())));
        assert_eq!(
            parse_target(&format!("12@{}", sha.to_uppercase())),
            Some((12, sha.clone()))
        );
        for bad in [
            "12",
            "12@abc",
            "x@",
            &format!("x@{sha}"),
            &format!("1@{sha}0"),
        ] {
            assert_eq!(parse_target(bad), None, "{bad}");
        }
        assert_eq!(approve_command(&[]), None);
        assert_eq!(
            approve_command(&[(1, "a".into()), (2, "b".into())]).as_deref(),
            Some("bosun approve 1@a 2@b")
        );
    }

    #[test]
    fn the_owner_token_goes_to_https_or_loopback_only() {
        for ok in [
            "https://api.github.com",
            "http://127.0.0.1:8080",
            "http://127.0.0.1",
            "http://localhost:9/x",
        ] {
            assert!(api_url_ok(ok), "{ok}");
        }
        for bad in [
            "http://api.github.com",
            "http://127.0.0.1.evil.com",
            "http://localhost.evil.com",
            "ftp://x",
            "",
        ] {
            assert!(!api_url_ok(bad), "{bad}");
        }
    }
}
