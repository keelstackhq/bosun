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
    let risk = status(statuses, "risk-check");
    // A run in flight or one that could not finish holds the PR: the owner board shows it.
    if risk.is_some_and(|s| matches!(s.state.as_str(), "pending" | "error")) {
        return Readiness::Waiting(
            "risk-check pending or errored: comment `/bosun recheck` (or re-run the risk workflow) if it stays",
        );
    }
    let waiting =
        risk.filter(|s| s.state == "failure" && s.description.starts_with(WAITING_PREFIX));
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

/// Most PRs one owner message lists; the rest are only counted.
const MESSAGE_PRS: usize = 10;
/// Longest title in the owner message, in characters.
const MESSAGE_TITLE: usize = 50;

/// The owner's chat message (design F7) for a ready list of `(pr, head_sha, title)`.
pub struct BatchMessage {
    /// First 8 hex characters of the SHA-256 of the shown pairs' sorted `<pr>@<head>\n` lines.
    pub id: String,
    pub text: String,
    /// The shown `<pr>@<full head>` pairs, in the order shown: the binding of the message.
    pub pairs: Vec<String>,
}

/// The message id of shown `<pr>@<head>` pairs.
pub fn message_id(pairs: &[String]) -> String {
    let mut lines: Vec<String> = pairs.iter().map(|p| format!("{p}\n")).collect();
    lines.sort();
    let digest = ring::digest::digest(&ring::digest::SHA256, lines.concat().as_bytes());
    digest.as_ref()[..4]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The chat message for `ready`; titles are shown inert. `None` when nothing is ready.
pub fn batch_message(ready: &[(u64, String, String)]) -> Option<BatchMessage> {
    if ready.is_empty() {
        return None;
    }
    let shown = &ready[..ready.len().min(MESSAGE_PRS)];
    let pairs: Vec<String> = shown.iter().map(|(pr, h, _)| format!("{pr}@{h}")).collect();
    let id = message_id(&pairs);
    let mut text = if ready.len() == 1 {
        format!("1 PR waits for your approval (message {id}):")
    } else {
        format!("{} PRs wait for your approval (message {id}):", ready.len())
    };
    for (pr, head, title) in shown {
        let short: String = head.chars().take(7).collect();
        let title = crate::core::text::inert(title, MESSAGE_TITLE);
        text.push_str(&format!("\n- #{pr}@{short}"));
        if !title.is_empty() {
            text.push(' ');
            text.push_str(&title);
        }
    }
    if ready.len() > shown.len() {
        text.push_str(&format!("\n- … and {} more", ready.len() - shown.len()));
    }
    let nums: Vec<String> = shown.iter().map(|(pr, ..)| pr.to_string()).collect();
    text.push_str(&format!("\nReply: approve {}", nums.join(" ")));
    Some(BatchMessage { id, text, pairs })
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

/// What a signed approval comment's first line says it is; each form signs its own message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// `/bosun approve <head>`.
    V1,
    /// `/bosun approve <head> after-review <id>`: bound to a dismissed owner review.
    V2(u64),
    /// `/bosun approve <head> bootstrap`: an owner key in bootstrap mode (D-012).
    Bootstrap,
    /// `/bosun approve <head> autonomous`: an owner key under the autonomous policy (D-017).
    Autonomous,
}

/// The form of a comment's first line for `head`: a v2 review id is plain decimal digits without
/// a leading zero, else `None` (fail closed).
fn form(first: &str, head: &str) -> Option<Form> {
    let rest = first.strip_prefix(&format!("/bosun approve {head}"))?;
    match rest {
        "" => return Some(Form::V1),
        " bootstrap" => return Some(Form::Bootstrap),
        " autonomous" => return Some(Form::Autonomous),
        _ => {}
    }
    let id = rest.strip_prefix(" after-review ")?;
    let n: u64 = id.parse().ok()?;
    (n.to_string() == id).then_some(Form::V2(n))
}

/// The `(form, key id, base64 signature)` of each `bosun-sig ed25519 <id> <sig>` line of a
/// comment whose first line has a `Form`; empty for any other comment.
pub fn signature_lines<'a>(body: &'a str, head: &str) -> Vec<(Form, &'a str, &'a str)> {
    let mut lines = body.lines();
    let first = lines.next().unwrap_or_default().trim();
    if head.len() != 40 {
        return Vec::new();
    }
    let Some(form) = form(first, head) else {
        return Vec::new();
    };
    lines
        .filter_map(|l| {
            let parts: Vec<&str> = l.trim().split(' ').collect();
            match parts.as_slice() {
                ["bosun-sig", "ed25519", id, sig] if !id.is_empty() && !sig.is_empty() => {
                    Some((form, *id, *sig))
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

/// The bytes an owner key signs in bootstrap mode (D-012): its own prefix, so it never verifies
/// as an owner v1 or v2 signature, nor they as it.
pub fn signed_message_bootstrap(owner: &str, repo: &str, pr: u64, head: &str) -> String {
    let (o, r) = (owner.to_ascii_lowercase(), repo.to_ascii_lowercase());
    format!("bosun-bootstrap v1 {o}/{r}#{pr} {head}")
}

/// The bytes an owner key signs under the autonomous policy (D-017): its own prefix again, so it
/// never verifies as an owner approval of any other form.
pub fn signed_message_autonomous(owner: &str, repo: &str, pr: u64, head: &str) -> String {
    let (o, r) = (owner.to_ascii_lowercase(), repo.to_ascii_lowercase());
    format!("bosun-autonomous v1 {o}/{r}#{pr} {head}")
}

/// The line a bootstrap-mode comment carries, for the reader; the first line is the marker.
pub fn bootstrap_line(until: &str) -> String {
    format!("Signed in bootstrap mode (D-012, until {until})")
}

/// The note line of an autonomous-mode signature (D-017).
pub const AUTONOMOUS_LINE: &str = "Signed in autonomous mode (D-017), not an owner answer";

/// `[approvals] policy`: who may approve gated PRs once bootstrap mode (D-012) is over (D-014).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Policy {
    /// Only the owner approves; the default, so a repo without the key fails safe.
    #[default]
    Owner,
    /// The orchestrator approves R-ci changes outside the CI workflows; the owner the rest.
    OwnerGated,
    /// The orchestrator approves everything the exclusions leave to a signature.
    Orchestrator,
    /// As `Orchestrator`, gated PRs included, but only with a clean independent verdict on the
    /// head (D-017); releases, environments and secrets stay with the owner.
    Autonomous,
}

impl Policy {
    pub fn name(self) -> &'static str {
        match self {
            Policy::Owner => "owner",
            Policy::OwnerGated => "owner-gated",
            Policy::Orchestrator => "orchestrator",
            Policy::Autonomous => "autonomous",
        }
    }
}

/// D-012's last day; no `bootstrap_until` later than this widens the window.
pub const BOOTSTRAP_END: &str = "2026-11-30";
/// CI workflow files of every provider R-ci names: owner-class in every policy, in the window too.
/// Their permissions, environments and publish steps cannot be judged reliably from a diff.
pub const WORKFLOW_PATHS: [&str; 13] = [
    ".github/workflows/**",
    ".github/actions/**",
    // Local composite actions (`uses: ./path`) run inside a workflow.
    "**/action.yml",
    "**/action.yaml",
    ".circleci/**",
    ".gitlab-ci.yml",
    ".gitlab/ci/**",
    ".buildkite/**",
    "**/jenkinsfile",
    "azure-pipelines.y*ml",
    "bitbucket-pipelines.yml",
    ".travis.yml",
    ".drone.yml",
];

/// What decides whether an orchestrator approval (the bootstrap-form signature) counts on a PR.
pub struct OrchestratorCase<'a> {
    pub policy: Policy,
    /// `[approvals] bootstrap_until` at base, already validated.
    pub bootstrap_until: Option<&'a str>,
    /// `YYYY-MM-DD`, UTC.
    pub today: &'a str,
    /// The name of the PR's approval requirement.
    pub requirement: &'a str,
    /// `risk::bootstrap_excluded` output: always the owner.
    pub excluded: &'a [String],
    /// The ids of every matched rule.
    pub rules: &'a [&'a str],
    /// Changed paths (previous names too) in [`WORKFLOW_PATHS`].
    pub workflows: &'a [String],
    /// `risk::autonomous_excluded` output: owner-only under [`Policy::Autonomous`].
    pub autonomous_excluded: &'a [String],
    /// The PR's capabilities (labels and linked task metadata, plus `weaken` for findings).
    pub capabilities: &'a [String],
}

/// `Ok(label for the status)` when an orchestrator approval can count, else `Err(why)`.
pub fn orchestrator_counts(c: &OrchestratorCase) -> Result<String, String> {
    // A weakened gate is the owner's under every policy and in the window (owner decision).
    if c.rules.contains(&"R-weaken") {
        return Err("a PR that weakens a gate (R-weaken) always needs the owner".into());
    }
    // The capability counts too, whatever rule id carries it.
    if c.capabilities.iter().any(|x| x == "weaken") {
        return Err("a PR with the weaken capability always needs the owner".into());
    }
    // D-017 decides first: its own exclusions only, and risk-check adds the verdict condition.
    if c.policy == Policy::Autonomous {
        if c.autonomous_excluded.is_empty() {
            return Ok("autonomous mode, D-017".into());
        }
        return Err(format!(
            "autonomous mode (D-017) never covers {}: the owner approves",
            c.autonomous_excluded.join(", ")
        ));
    }
    if c.requirement != "owner" {
        return Err(format!(
            "the PR needs {}, not the owner class",
            c.requirement
        ));
    }
    if !c.excluded.is_empty() {
        return Err(format!(
            "the PR touches {}, which always needs the owner",
            c.excluded.join(", ")
        ));
    }
    if !c.workflows.is_empty() {
        return Err(format!(
            "the PR changes {}: a CI workflow always needs the owner (D-014)",
            c.workflows.join(", ")
        ));
    }
    let end = c.bootstrap_until.map(|u| u.min(BOOTSTRAP_END));
    if end.is_some_and(|e| c.today <= e) {
        // The rest of R-gov (hook settings, REVIEW.md, plans, agent instructions) and production
        // writes are not `BOOTSTRAP_EXCLUDED` paths, yet the window never covers them either.
        let gated: Vec<&str> = (c.rules.iter().copied())
            .filter(|r| ["R-gov", "R-prod-write"].contains(r))
            .collect();
        if !gated.is_empty() {
            return Err(format!(
                "bootstrap mode (D-012) never covers {}: the owner approves",
                gated.join(", ")
            ));
        }
        return Ok("bootstrap mode, D-012".into());
    }
    let window = match end {
        Some(e) => format!("bootstrap mode ended on {e}, D-012"),
        None => "bootstrap mode is off: `[approvals] bootstrap_until` is not set, D-012".into(),
    };
    let label = |p: Policy| Ok(format!("policy {}, D-014", p.name()));
    match c.policy {
        Policy::Orchestrator => label(c.policy),
        Policy::Autonomous => unreachable!("decided above"),
        Policy::Owner => Err(format!(
            "approval policy owner: only the owner approves ({window})"
        )),
        Policy::OwnerGated => {
            let other: Vec<&str> = (c.rules.iter().copied()).filter(|r| *r != "R-ci").collect();
            if other.is_empty() {
                return label(c.policy);
            }
            Err(format!(
                "approval policy owner-gated: only R-ci changes outside workflows; {} need the owner ({window})",
                other.join(", ")
            ))
        }
    }
}

/// The line a comment signed under an approval policy (not the bootstrap window) carries.
pub fn policy_line(p: Policy) -> String {
    format!("Signed under approval policy {} (D-014)", p.name())
}

/// The trusted `risk-check` status waits for an owner and lists R-ci alone.
pub fn only_r_ci(statuses: &[Stat]) -> bool {
    let waiting = status(statuses, "risk-check")
        .filter(|s| s.state == "failure" && s.description.starts_with(WAITING_PREFIX));
    waiting.is_some_and(|s| risk_ids(&s.description) == ["R-ci"])
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

/// The comment `bosun approve --sign-key` posts; `note` is a line for the reader.
pub fn signed_comment(
    head: &str,
    form: Form,
    note: Option<&str>,
    key_id: &str,
    sig_b64: &str,
) -> String {
    let tail = match form {
        Form::V1 => String::new(),
        Form::V2(r) => format!(" after-review {r}"),
        Form::Bootstrap => " bootstrap".into(),
        Form::Autonomous => " autonomous".into(),
    };
    let note = note.map_or(String::new(), |n| format!("{n}\n"));
    format!("/bosun approve {head}{tail}\n{note}bosun-sig ed25519 {key_id} {sig_b64}")
}

#[cfg(test)]
mod tests {

    fn case<'a>(policy: Policy, until: Option<&'a str>, today: &'a str) -> OrchestratorCase<'a> {
        OrchestratorCase {
            policy,
            bootstrap_until: until,
            today,
            requirement: "owner",
            excluded: &[],
            rules: &["R-ci"],
            workflows: &[],
            autonomous_excluded: &[],
            capabilities: &[],
        }
    }

    #[test]
    fn autonomous_decides_first_and_only_the_exclusions_stop_it() {
        let w = vec![".github/workflows/ci.yml".to_string()];
        let mut c = case(Policy::Autonomous, None, "2027-01-01");
        c.requirement = "two-verdicts";
        c.excluded = &w;
        c.workflows = &w;
        assert_eq!(orchestrator_counts(&c).unwrap(), "autonomous mode, D-017");
        let x = vec!["hoist.toml".to_string(), "R-secrets".to_string()];
        c.autonomous_excluded = &x;
        let why = orchestrator_counts(&c).unwrap_err();
        assert!(
            why.contains("D-017") && why.contains("hoist.toml, R-secrets"),
            "{why}"
        );
        // Other policies ignore the field.
        let mut c = case(Policy::Orchestrator, None, "2027-01-01");
        c.autonomous_excluded = &x;
        assert_eq!(
            orchestrator_counts(&c).unwrap(),
            "policy orchestrator, D-014"
        );
        assert_eq!(Policy::Autonomous.name(), "autonomous");
    }

    #[test]
    fn an_autonomous_waiting_status_is_not_an_owner_wait() {
        let mut s = base();
        s[0].description = "high risk, autonomous mode (D-017): clean verdict and orchestrator signature needed: R-ci".into();
        assert_eq!(readiness(&s, &[]), Readiness::NotWaiting);
    }

    #[test]
    fn r_weaken_never_counts_for_the_orchestrator() {
        let rules = ["R-ci", "R-weaken"];
        for p in [
            Policy::Owner,
            Policy::OwnerGated,
            Policy::Orchestrator,
            Policy::Autonomous,
        ] {
            for (until, today) in [(Some("2026-11-30"), "2026-11-10"), (None, "2026-12-01")] {
                let mut c = case(p, until, today);
                c.rules = &rules;
                let why = orchestrator_counts(&c).unwrap_err();
                assert!(why.contains("R-weaken"), "{p:?} {until:?}: {why}");
            }
        }
    }

    #[test]
    fn the_weaken_capability_never_counts_for_the_orchestrator() {
        let caps = vec!["weaken".to_string()];
        for p in [Policy::Orchestrator, Policy::Autonomous] {
            let mut c = case(p, Some("2026-11-30"), "2026-11-10");
            c.rules = &["R-gate-loosen"];
            c.capabilities = &caps;
            let why = orchestrator_counts(&c).unwrap_err();
            assert!(why.contains("weaken"), "{p:?}: {why}");
        }
    }

    #[test]
    fn the_window_counts_through_its_last_day_and_is_capped() {
        let ok = |c: &OrchestratorCase| orchestrator_counts(c).unwrap();
        let c = case(Policy::Owner, Some("2026-11-30"), "2026-11-30");
        assert_eq!(ok(&c), "bootstrap mode, D-012");
        let c = case(Policy::Owner, Some("2027-06-30"), "2026-11-30");
        assert_eq!(ok(&c), "bootstrap mode, D-012");
        let c = case(Policy::Owner, Some("2027-06-30"), "2026-12-01");
        let why = orchestrator_counts(&c).unwrap_err();
        assert!(why.contains("2026-11-30") && why.contains("D-012"), "{why}");
        let c = case(Policy::Owner, None, "2026-11-10");
        assert!(orchestrator_counts(&c).unwrap_err().contains("is off"));
    }

    #[test]
    fn the_window_never_covers_the_rest_of_r_gov_or_production_writes() {
        for r in ["R-gov", "R-prod-write"] {
            let mut c = case(Policy::Owner, Some("2026-11-30"), "2026-11-10");
            let rules = ["R-ci", r];
            c.rules = &rules;
            let why = orchestrator_counts(&c).unwrap_err();
            assert!(why.contains(r) && why.contains("D-012"), "{why}");
            // After the window the policy decides, as before.
            let mut c = case(Policy::Orchestrator, None, "2026-12-01");
            c.rules = &rules;
            assert!(orchestrator_counts(&c).is_ok(), "{r}");
        }
    }

    #[test]
    fn after_the_window_the_policy_decides() {
        let c = case(Policy::OwnerGated, None, "2026-12-01");
        assert_eq!(
            orchestrator_counts(&c).unwrap(),
            "policy owner-gated, D-014"
        );
        let mut c = case(Policy::OwnerGated, None, "2026-12-01");
        c.rules = &["R-ci", "R-gov"];
        assert!(orchestrator_counts(&c).unwrap_err().contains("R-gov"));
        let w = vec![".github/workflows/ci.yml".to_string()];
        let mut c = case(Policy::OwnerGated, None, "2026-12-01");
        c.workflows = &w;
        assert!(orchestrator_counts(&c).unwrap_err().contains("ci.yml"));
        let mut c = case(Policy::Orchestrator, None, "2026-12-01");
        c.rules = &["R-gov"];
        assert_eq!(
            orchestrator_counts(&c).unwrap(),
            "policy orchestrator, D-014"
        );
        // A workflow always needs the owner, in every policy and in the window.
        c.workflows = &w;
        assert!(orchestrator_counts(&c).unwrap_err().contains("D-014"));
        let mut c = case(Policy::Orchestrator, Some("2026-11-30"), "2026-11-10");
        c.workflows = &w;
        assert!(orchestrator_counts(&c).unwrap_err().contains("ci.yml"));
        assert_eq!(Policy::default(), Policy::Owner);
        let c = case(Policy::Owner, None, "2026-12-01");
        assert!(
            orchestrator_counts(&c)
                .unwrap_err()
                .contains("only the owner")
        );
    }

    #[test]
    fn excluded_paths_and_other_classes_never_count() {
        let ex = vec!["hoist.toml".to_string()];
        for p in [Policy::Owner, Policy::OwnerGated, Policy::Orchestrator] {
            let mut c = case(p, Some("2026-11-30"), "2026-11-10");
            c.excluded = &ex;
            assert!(orchestrator_counts(&c).unwrap_err().contains("hoist.toml"));
            let mut c = case(p, Some("2026-11-30"), "2026-11-10");
            c.requirement = "two-verdicts";
            assert!(
                orchestrator_counts(&c)
                    .unwrap_err()
                    .contains("two-verdicts")
            );
        }
    }
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
    fn a_pending_or_errored_risk_check_waits_on_a_rerun() {
        for state in ["pending", "error"] {
            let s = [st("risk-check", state, "risk-check is judging this head")];
            assert_eq!(
                readiness(&s, &[]),
                Readiness::Waiting(
                    "risk-check pending or errored: comment `/bosun recheck` (or re-run the risk workflow) if it stays"
                ),
                "{state}"
            );
        }
        // Only the Actions token's status counts, as for every other state.
        let forged = [by("risk-check", "error", "x", "mallory")];
        assert_eq!(readiness(&forged, &[]), Readiness::NotWaiting);
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

    fn item(pr: u64, head: &str, title: &str) -> (u64, String, String) {
        (pr, head.into(), title.into())
    }

    #[test]
    fn the_message_id_is_a_known_answer() {
        // printf '12@<40 a>\n3@<40 b>\n' | sha256sum | cut -c1-8   ("12@" sorts before "3@")
        let pairs = vec![
            format!("3@{}", "b".repeat(40)),
            format!("12@{}", "a".repeat(40)),
        ];
        assert_eq!(message_id(&pairs), "77926130");
    }

    #[test]
    fn the_batch_message_binds_the_shown_pairs() {
        assert!(batch_message(&[]).is_none());
        let h = "abcdef0123".repeat(4);
        let m = batch_message(&[item(1, &h, "`**x**`"), item(2, &h, " ")]).unwrap();
        assert_eq!(m.pairs, vec![format!("1@{h}"), format!("2@{h}")]);
        assert_eq!(m.id, message_id(&m.pairs));
        assert!(
            m.text
                .contains("\n- #1@abcdef0 `ˋ**x**ˋ`\n- #2@abcdef0\nReply")
        );
        let many: Vec<_> = (1..=11).map(|n| item(n, &h, "t")).collect();
        let m = batch_message(&many).unwrap();
        assert_eq!(m.pairs.len(), 10);
        assert!(m.text.contains("- … and 1 more\n"));
        assert!(m.text.ends_with("Reply: approve 1 2 3 4 5 6 7 8 9 10"));
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
        assert_eq!(v1, [(Form::V1, "k1", "AAAA")]);
        let b2 = format!("/bosun approve {h} after-review 42\n{sig}");
        let v2 = signature_lines(&b2, &h);
        assert_eq!(v2, [(Form::V2(42), "k1", "AAAA")]);
        let b3 = format!(
            "/bosun approve {h} bootstrap\n{}\n{sig}",
            bootstrap_line("2026-11-30")
        );
        assert_eq!(signature_lines(&b3, &h), [(Form::Bootstrap, "k1", "AAAA")]);
        let b4 = format!("/bosun approve {h} autonomous\n{sig}");
        assert_eq!(signature_lines(&b4, &h), [(Form::Autonomous, "k1", "AAAA")]);
        assert_eq!(
            signed_message_autonomous("Acme", "Widget", 3, &h),
            format!("bosun-autonomous v1 acme/widget#3 {h}")
        );
        for bad in ["", "x", "07", "-1", "1 2", "99999999999999999999999"] {
            let body = format!("/bosun approve {h} after-review {bad}\n{sig}");
            assert!(signature_lines(&body, &h).is_empty(), "{bad:?}");
            let body = format!("/bosun approve {h} bootstrap {bad}x\n{sig}");
            assert!(signature_lines(&body, &h).is_empty(), "{bad:?}");
        }
        assert_eq!(
            signed_message_v2("Acme", "Widget", 3, &h, 7),
            format!("bosun-approve v2 acme/widget#3 {h} after-review 7")
        );
        assert_eq!(
            signed_message_bootstrap("Acme", "Widget", 3, &h),
            format!("bosun-bootstrap v1 acme/widget#3 {h}")
        );
    }
}
