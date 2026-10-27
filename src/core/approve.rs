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

/// Whether this is a Claude cloud session, whose agent proxy rewrites GitHub writes (app
/// attribution and a footer): `CCR_AGENT_PROXY_ENABLED` is `1`/`true`, or
/// `CLAUDE_CODE_REMOTE_ENVIRONMENT_TYPE` starts with `cloud`.
pub fn in_cloud(proxy_enabled: Option<&str>, environment_type: Option<&str>) -> bool {
    let proxy = proxy_enabled.map(str::trim);
    proxy.is_some_and(|v| {
        v == "1"
            || ["true", "yes", "on"]
                .iter()
                .any(|t| v.eq_ignore_ascii_case(t))
    }) || environment_type.is_some_and(|v| v.trim().to_ascii_lowercase().starts_with("cloud"))
}

/// The owner token only goes to https, or to a loopback host (the tests' fake).
pub fn api_url_ok(url: &str) -> bool {
    let loopback = |host: &str| {
        url.strip_prefix(host)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
    };
    url.starts_with("https://") || loopback("http://127.0.0.1") || loopback("http://localhost")
}

/// Where risk-check reads the enrolled approval keys: the default branch, never a PR's head.
pub const APPROVERS_PATH: &str = ".bosun/approvers.toml";

/// One enrolled key of `.bosun/approvers.toml`: `ed25519` is the base64 raw public key.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApproverKey {
    pub id: String,
    pub ed25519: String,
    /// `owner` (the default) or `delegate`.
    #[serde(default)]
    pub role: Role,
}

/// What a key's signature stands for. Each role signs its own message, so a signature of one
/// role never counts as the other.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    Owner,
    Delegate,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Delegate => "delegate",
        }
    }

    /// The first word of the signed message.
    fn prefix(self) -> &'static str {
        match self {
            Role::Owner => "bosun-approve",
            Role::Delegate => "bosun-delegate",
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ApproversFile {
    #[serde(default)]
    key: Vec<ApproverKey>,
}

/// Parse `.bosun/approvers.toml`. Ids must be unique tokens without whitespace.
pub fn parse_approvers(text: &str) -> Result<Vec<ApproverKey>, String> {
    let file: ApproversFile = toml::from_str(text).map_err(|e| e.message().to_string())?;
    for (i, k) in file.key.iter().enumerate() {
        if k.id.is_empty() || k.id.chars().any(char::is_whitespace) {
            return Err(format!("key id {:?} is empty or has whitespace", k.id));
        }
        if file.key[..i].iter().any(|o| o.id == k.id) {
            return Err(format!("key id {:?} is listed twice", k.id));
        }
    }
    Ok(file.key)
}

/// The exact bytes a key of `role` signs. Owner and repo are lowercased (GitHub names are
/// case-insensitive), so the signer and risk-check agree however each spells them.
pub fn signed_message(role: Role, owner: &str, repo: &str, pr: u64, head: &str) -> String {
    format!(
        "{} v1 {}/{}#{pr} {head}",
        role.prefix(),
        owner.to_ascii_lowercase(),
        repo.to_ascii_lowercase()
    )
}

/// The review id a v2 comment names, from the first line `/bosun approve <head> after-review <id>`:
/// plain decimal digits without a leading zero, else `None` (fail closed).
fn after_review(first: &str, head: &str) -> Option<u64> {
    let id = first
        .strip_prefix(&format!("/bosun approve {head} after-review "))?
        .trim_end();
    let n: u64 = id.parse().ok()?;
    (n.to_string() == id).then_some(n)
}

/// The `(review id, key id, base64 signature)` of each `bosun-sig ed25519 <id> <sig>` line of a
/// comment whose first line is `/bosun approve <head>` (review id `None`: a v1 signature) or
/// `/bosun approve <head> after-review <id>` (a v2 signature); empty for any other comment.
pub fn signature_lines<'a>(body: &'a str, head: &str) -> Vec<(Option<u64>, &'a str, &'a str)> {
    let mut lines = body.lines();
    let first = lines.next().unwrap_or_default().trim();
    if head.len() != 40 {
        return Vec::new();
    }
    let review = if first == format!("/bosun approve {head}") {
        None
    } else if let Some(n) = after_review(first, head) {
        Some(n)
    } else {
        return Vec::new();
    };
    lines
        .filter_map(|l| {
            let parts: Vec<&str> = l.trim().split(' ').collect();
            match parts.as_slice() {
                ["bosun-sig", "ed25519", id, sig] if !id.is_empty() && !sig.is_empty() => {
                    Some((review, *id, *sig))
                }
                _ => None,
            }
        })
        .collect()
}

/// Whether `sig` is a valid ed25519 signature of `msg` by the raw 32-byte `public_key`.
pub fn verify_ed25519(public_key: &[u8], msg: &[u8], sig: &[u8]) -> bool {
    use ring::signature::{ED25519, UnparsedPublicKey};
    public_key.len() == 32
        && sig.len() == 64
        && UnparsedPublicKey::new(&ED25519, public_key)
            .verify(msg, sig)
            .is_ok()
}

/// The bytes an owner key signs to clear the withdrawal of the dismissed owner review `review`:
/// bound to that review, so it cannot be replayed after a newer dismissal.
pub fn signed_message_v2(owner: &str, repo: &str, pr: u64, head: &str, review: u64) -> String {
    format!(
        "bosun-approve v2 {}/{}#{pr} {head} after-review {review}",
        owner.to_ascii_lowercase(),
        repo.to_ascii_lowercase()
    )
}

/// Files that decide who approves: a PR that changes one (or renames one away) never passes on a
/// signed approval, owner or delegate key, so a compromised key cannot enroll keys or owners.
pub const TRUST_FILES: [&str; 5] = [
    APPROVERS_PATH,
    "governance/owners.toml",
    "CODEOWNERS",
    ".github/CODEOWNERS",
    "docs/CODEOWNERS",
];

/// The changed trust files among `(path, previous path)` pairs.
pub fn trust_files_changed<'a>(
    files: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
) -> Vec<String> {
    files
        .into_iter()
        .filter(|(p, prev)| {
            std::iter::once(*p)
                .chain(*prev)
                .any(|x| TRUST_FILES.contains(&x))
        })
        .map(|(p, _)| p.to_string())
        .collect()
}

/// The comment `bosun approve --sign-key` posts; `after` is the dismissed review a v2 names.
pub fn signed_comment(head: &str, after: Option<u64>, key_id: &str, sig_b64: &str) -> String {
    let after = after.map_or(String::new(), |r| format!(" after-review {r}"));
    format!("/bosun approve {head}{after}\nbosun-sig ed25519 {key_id} {sig_b64}")
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
        let unlinked = "high risk, owner approval needed (links no task, so verdicts cannot count): R-gov, R-ci-review";
        assert!(unlinked.starts_with(WAITING_PREFIX));
        assert_eq!(risk_ids(unlinked), ["R-gov", "R-ci-review"]);
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
    fn cloud_sessions_are_told_by_two_variables() {
        assert!(in_cloud(Some("1"), None));
        for v in ["True", "YES", "on", " 1 "] {
            assert!(in_cloud(Some(v), None), "{v}");
        }
        assert!(in_cloud(None, Some("Cloud_default")));
        assert!(!in_cloud(Some("off"), Some("local")));
        assert!(in_cloud(None, Some("cloud_default")));
        assert!(!in_cloud(Some("0"), Some("local")));
        assert!(!in_cloud(Some(""), Some("")));
        assert!(!in_cloud(None, None));
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

    #[test]
    fn signature_lines_tell_v1_from_v2() {
        let h = "a".repeat(40);
        let sig = "bosun-sig ed25519 k1 AAAA";
        let b1 = format!("/bosun approve {h}\n{sig}");
        let v1 = signature_lines(&b1, &h);
        assert_eq!(v1, [(None, "k1", "AAAA")]);
        let b2 = format!("/bosun approve {h} after-review 42\n{sig}");
        let v2 = signature_lines(&b2, &h);
        assert_eq!(v2, [(Some(42), "k1", "AAAA")]);
        for bad in ["", "x", "07", "-1", "1 2", "99999999999999999999999"] {
            let body = format!("/bosun approve {h} after-review {bad}\n{sig}");
            assert!(signature_lines(&body, &h).is_empty(), "{bad:?}");
        }
        assert_eq!(
            signed_message_v2("Acme", "Widget", 3, &h, 7),
            format!("bosun-approve v2 acme/widget#3 {h} after-review 7")
        );
    }
}
