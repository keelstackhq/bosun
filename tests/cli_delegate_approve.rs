//! Delegated decisions (spec §8): a delegate key's signed `bosun-delegate v1` approval passes a
//! PR whose matched rules are all `approval = "delegate"`, with one independent approving
//! verdict without major findings on the head. Critical changes always need the owner.
mod common;

use common::fake::FakeGithub;
use common::{bosun, lease_note, one_object, run};
use serde_json::{Value, json};

const NOW: &str = "2026-10-22T10:00:00Z";
const BASE: &str = "base0000";
const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
const OLD: &str = "1111111111111111111111111111111111111111";
const MAIN: &str = "refs/heads/main";
const OWNERS: &str = "[[owner]]\ngithub = \"ada\"\n";
const TASK: &str = "<!-- bosun:task {} -->";

// RFC 8032 test 1 key (role owner) and test 2 key (role delegate).
const PUB_OWNER: &str = "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=";
const PUB_DELEGATE: &str = "PUAXw+hDiVqStwqnTRt+vJyYLM8uxJaMwM1V8Sr0Zgw=";
const PEM_DELEGATE: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIEzNCJso/5banbbDRuwRTg9bijGfNaumJNqM9u1PuKb7\n-----END PRIVATE KEY-----\n";
const PEM_OWNER: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJ1hsZ3v/VpguoRK9JLsLMREScVpezJpGXA7rAMcrn9g\n-----END PRIVATE KEY-----\n";

// `openssl pkeyutl -sign -rawin`, the message as printf '%s' writes it.
/// Delegate key over `bosun-delegate v1 acme/widget#1 <HEAD>`.
const SIG_DELEGATE: &str =
    "Yx495gcWgyARd5zAlqvj3wunBoTuAcBXFjw+nbz+G+fMohcZhSAJM/kiYFTRzdsQCBgn+uuZSUt2qkwVB5EHBg==";
/// Delegate key over the owner message `bosun-approve v1 acme/widget#1 <HEAD>`.
const SIG_DELEGATE_OWNER_MSG: &str =
    "QaSlWA89SY/lbC3KRhb7YMRUCs8twG1ipEnTH1RI7j3C6YmLPO5fj5lhr5Sm4hBYozbu0XKkPfWAOunh4IYfBQ==";
/// Owner key over the delegate message.
const SIG_OWNER_DELEGATE_MSG: &str =
    "Lu1kCW5Hlh+sC9tIOV0eQp1lP1K7XudaCDYhPPWkISfDnG3/T6Xs4rckRo0L7CxdoHmYM/VwmwDKR5BVKHnWAg==";

const APPROVERS: &str = "[[key]]\nid = \"o1\"\ned25519 = \"11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=\"\nrole = \"owner\"\n\n[[key]]\nid = \"d1\"\ned25519 = \"PUAXw+hDiVqStwqnTRt+vJyYLM8uxJaMwM1V8Sr0Zgw=\"\nrole = \"delegate\"\n";

/// R-gov stays owner; R-gate and R-ci are delegate; R-secrets is configured delegate but is
/// critical whatever the file says.
const RULES: &str = r#"
[[rule]]
id = "R-gov"
why = "gov"
paths = [".bosun/**", "governance/**"]

[[rule]]
id = "R-ci"
why = "ci"
paths = [".github/workflows/**", "deny.toml"]
approval = "delegate"

[[rule]]
id = "R-gate"
why = "gate"
paths = ["src/**", "Cargo.toml", "docs/moved/**"]
approval = "delegate"

[[rule]]
id = "R-secrets"
why = "secrets"
content = "SECRET"
approval = "delegate"
"#;

fn sig_comment(id: &str, sig: &str) -> String {
    format!("/bosun approve {HEAD}\nbosun-sig ed25519 {id} {sig}\n\n---\n_footer_")
}

fn verdict(
    session: &str,
    head: &str,
    approved: bool,
    confidence: f64,
    major: Option<u32>,
) -> String {
    let mut r = json!({"bosun": "verdict", "session": session, "head_sha": head,
        "approved": approved, "confidence": confidence});
    if let Some(m) = major {
        r["major"] = json!(m);
    }
    format!("Review verdict: approve {confidence}, 0 blocker.\n\n{r}")
}

/// PR 1 (Closes #7) at HEAD, changing `path`; worker `session_w` holds the task's lease.
fn pr(path: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "Closes #7");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    fake.issue(7, "task", &[], TASK);
    fake.comment(
        7,
        &lease_note("session_w", "2026-10-22T12:00:00Z", "src/**"),
    );
    fake.content("governance/owners.toml", BASE, OWNERS);
    fake.content(".bosun/risk.toml", BASE, RULES);
    fake.content(".bosun/approvers.toml", MAIN, APPROVERS);
    fake.pull_file(1, path, "modified", 1, 0, Some("@@ -0,0 +1,1 @@\n+x"));
    fake
}

/// `pr(path)` with a good verdict by `session_r` and the delegate's signature.
fn delegated(path: &str) -> FakeGithub {
    let fake = pr(path);
    fake.comment_by(
        1,
        "github-actions[bot]",
        &verdict("session_r", HEAD, true, 0.95, Some(0)),
    );
    fake.comment_by(1, "claude", &sig_comment("d1", SIG_DELEGATE));
    fake.last_comment(|c| c.via_app = Some("claude".into()));
    fake
}

fn check(fake: &FakeGithub) -> (i32, Value) {
    run(fake, "session_test", NOW, &["risk-check", "1"])
}

fn description(fake: &FakeGithub) -> String {
    let s = fake.statuses_of(HEAD);
    let s = s
        .iter()
        .rev()
        .find(|s| s["context"] == "risk-check")
        .unwrap();
    s["description"].as_str().unwrap().to_string()
}

#[test]
fn a_delegate_with_one_clean_verdict_approves_delegate_rules() {
    let fake = delegated("deny.toml");
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["approved_by"], "delegate d1");
    assert_eq!(v["data"]["requirement"], "delegate");
    assert!(
        description(&fake).ends_with("approved by delegate d1"),
        "{}",
        description(&fake)
    );

    // An owner's request for changes on the head withdraws it.
    fake.review(1, "ada", "CHANGES_REQUESTED", HEAD);
    assert_eq!(check(&fake).0, 5);
}

#[test]
fn waiting_for_a_delegate_still_reads_as_owner_approval_needed() {
    let fake = pr("deny.toml");
    assert_eq!(check(&fake).0, 5);
    assert!(
        description(&fake).starts_with("high risk, owner approval needed (or a delegate): R-ci"),
        "{}",
        description(&fake)
    );
}

#[test]
fn the_verdict_must_be_independent_clean_confident_and_on_the_head() {
    for (session, head, approved, confidence, major) in [
        ("session_r", HEAD, true, 0.95, Some(1)), // a major finding
        ("session_r", HEAD, true, 0.95, None),    // an old record without the count
        ("session_r", HEAD, true, 0.89, Some(0)), // not confident enough
        ("session_r", HEAD, false, 0.95, Some(0)),
        ("session_r", OLD, true, 0.95, Some(0)), // another head
        ("session_w", HEAD, true, 0.95, Some(0)), // the worker holds the lease
    ] {
        let fake = pr("deny.toml");
        fake.comment_by(
            1,
            "github-actions[bot]",
            &verdict(session, head, approved, confidence, major),
        );
        fake.comment_by(1, "claude", &sig_comment("d1", SIG_DELEGATE));
        assert_eq!(check(&fake).0, 5, "{session} {head} {confidence} {major:?}");
    }
    // A verdict not written by the verdict workflow does not count.
    let fake = pr("deny.toml");
    fake.comment_by(
        1,
        "mallory",
        &verdict("session_r", HEAD, true, 0.95, Some(0)),
    );
    fake.comment_by(1, "claude", &sig_comment("d1", SIG_DELEGATE));
    assert_eq!(check(&fake).0, 5);
}

#[test]
fn roles_and_messages_are_not_interchangeable() {
    // A delegate key signing the owner message is no owner approval, on an owner rule too.
    let fake = pr(".bosun/limits.toml");
    fake.comment_by(1, "claude", &sig_comment("d1", SIG_DELEGATE_OWNER_MSG));
    assert_eq!(check(&fake).0, 5);
    // An owner key signing the delegate message is neither.
    let fake = pr("deny.toml");
    fake.comment_by(
        1,
        "github-actions[bot]",
        &verdict("session_r", HEAD, true, 0.95, Some(0)),
    );
    fake.comment_by(1, "claude", &sig_comment("o1", SIG_OWNER_DELEGATE_MSG));
    assert_eq!(check(&fake).0, 5);
    // A delegate signature does not pass an owner rule, even with a verdict.
    let fake = delegated("governance/owners.toml");
    assert_eq!(check(&fake).0, 5);
}

#[test]
fn critical_changes_always_need_the_owner() {
    // Configured delegate, but the approval system itself: a hard floor.
    for path in [
        ".github/workflows/risk.yml",
        ".github/workflows/ci.yml",
        ".github/workflows/main-health.yml",
        "src/cli/risk_check.rs",
        "src/config.rs",
        "Cargo.toml",
    ] {
        let fake = delegated(path);
        // bosun's own code counts as critical where the base has it.
        fake.content("src/cli/risk_check.rs", BASE, "// risk-check");
        let (code, v) = check(&fake);
        assert_eq!(code, 5, "{path}: {v}");
        assert_eq!(v["data"]["requirement"], "owner", "{path}");
        assert!(
            description(&fake).contains("(critical: "),
            "{path}: {}",
            description(&fake)
        );
    }
    // A rename out of a critical path is still critical.
    let fake = delegated("docs/moved/approvers.toml");
    fake.renamed_from(1, ".bosun/approvers.toml");
    assert_eq!(check(&fake).0, 5, "rename out of .bosun");
    // R-secrets is critical whatever its approval says.
    let fake = pr("src/gates/x.rs");
    fake.state.lock().unwrap().files.get_mut(&1).unwrap()[0]["patch"] =
        json!("@@ -0,0 +1,1 @@\n+SECRET=1");
    fake.comment_by(
        1,
        "github-actions[bot]",
        &verdict("session_r", HEAD, true, 0.95, Some(0)),
    );
    fake.comment_by(1, "claude", &sig_comment("d1", SIG_DELEGATE));
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert!(
        description(&fake).contains("critical: R-secrets"),
        "{}",
        description(&fake)
    );
}

#[test]
fn bosun_code_is_delegate_able_where_the_base_has_no_bosun_code() {
    // In a repo that is not bosun, src/cli/risk_check.rs is ordinary code.
    let fake = delegated("src/cli/risk_check.rs");
    assert_eq!(check(&fake).0, 0);
}

#[test]
fn a_pr_without_a_task_cannot_be_delegated() {
    let fake = delegated("deny.toml");
    fake.pull_set(1, "body", json!(""));
    let (code, _) = check(&fake);
    assert_eq!(code, 5);
    assert!(
        description(&fake).contains("links no task"),
        "{}",
        description(&fake)
    );
}

#[test]
fn without_delegate_rules_the_mode_is_off() {
    let fake = delegated("deny.toml");
    fake.content(
        ".bosun/risk.toml",
        BASE,
        &RULES.replace("approval = \"delegate\"", ""),
    );
    let (code, v) = check(&fake);
    assert_eq!(code, 5);
    assert_eq!(v["data"]["requirement"], "owner");
}

fn key_file(pem: &str) -> std::path::PathBuf {
    let dir = tempfile::tempdir().unwrap().keep();
    let path = dir.join("key.pem");
    std::fs::write(&path, pem).unwrap();
    path
}

fn sign(fake: &FakeGithub, pem: &str, extra: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("CCR_AGENT_PROXY_ENABLED", "1")
        .args(["approve", &format!("1@{HEAD}"), "--print", "--sign-key"])
        .arg(key_file(pem))
        .args(extra)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

#[test]
fn approve_as_delegate_signs_the_delegate_message() {
    let fake = pr("deny.toml");
    let (code, v) = sign(&fake, PEM_DELEGATE, &["--as", "delegate"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["body"],
        format!("/bosun approve {HEAD}\nbosun-sig ed25519 d1 {SIG_DELEGATE}")
    );
    assert_eq!(v["data"]["role"], "delegate");
    // The role must match the enrolled one, both ways.
    assert_eq!(
        sign(&fake, PEM_DELEGATE, &[]).0,
        1,
        "a delegate key is no owner"
    );
    assert_eq!(sign(&fake, PEM_OWNER, &["--as", "delegate"]).0, 1);
    assert_eq!(sign(&fake, PEM_OWNER, &[]).0, 0);
    let _ = PUB_OWNER;
}

#[test]
fn a_mix_with_two_verdicts_needs_the_owner() {
    let fake = delegated("channel/x.toml");
    let rules = format!(
        "{RULES}\n[[rule]]\nid = \"R-ci-review\"\nwhy = \"c\"\npaths = [\"channel/**\"]\napproval = \"two-verdicts\"\n"
    );
    fake.content(".bosun/risk.toml", BASE, &rules);
    fake.pull_file(
        1,
        "deny.toml",
        "modified",
        1,
        0,
        Some("@@ -0,0 +1,1 @@\n+x"),
    );
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["requirement"], "owner");
}

#[test]
fn no_delegation_while_halted_off_main_or_without_a_real_leased_task() {
    // Halted: the owner stopped all work.
    let fake = delegated("deny.toml");
    fake.variable("BOSUN_HALT", "1");
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert!(
        v["data"]["delegate_blocked"]
            .as_str()
            .unwrap()
            .contains("halted"),
        "{v}"
    );

    // A PR into another branch.
    let fake = delegated("deny.toml");
    fake.pull_set(1, "base", json!({"sha": BASE, "ref": "release"}));
    assert_eq!(check(&fake).0, 5);

    // A decoy link: an issue that is not a bosun task, a closed task, a task without a lease.
    let fake = delegated("deny.toml");
    fake.pull_set(1, "body", json!("Closes #8"));
    fake.issue(8, "decoy", &[], "");
    assert_eq!(check(&fake).0, 5, "not a task");
    fake.pull_set(1, "body", json!("Closes #9"));
    fake.issue(9, "task", &[], TASK);
    assert_eq!(check(&fake).0, 5, "no lease");
    fake.comment(9, &lease_note("session_w", "2026-10-22T12:00:00Z", "x"));
    assert_eq!(check(&fake).0, 0, "an open leased task");
    fake.close_issue(9, NOW);
    assert_eq!(check(&fake).0, 5, "a closed task");
}

#[test]
fn one_public_key_under_two_ids_is_malformed() {
    let fake = delegated("deny.toml");
    let twice = format!("{APPROVERS}\n[[key]]\nid = \"o2\"\ned25519 = \"{PUB_DELEGATE}\"\n");
    fake.content(".bosun/approvers.toml", MAIN, &twice);
    assert_eq!(check(&fake).0, 5);
}
