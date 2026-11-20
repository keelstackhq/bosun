//! `[approvals] policy = "quorum"` (D-022): an autonomous-form signature plus two counting
//! verdicts from different sessions passes on every path except the trust root, and
//! `bosun approve --bootstrap` signs only under the same conditions.
mod common;

use bosun::github::b64;
use common::fake::FakeGithub;
use common::{bosun, lease_note, one_object, run};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};

const BASE: &str = "base0000";
const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
const OTHER: &str = "89abcdef0123456789abcdef0123456789abcdef";
const MAIN: &str = "refs/heads/main";
const NOW: &str = "2026-12-05T10:00:00Z";
const PLAIN: &str = "@@ -0,0 +1,1 @@\n+x";
const GATE: &str = "src/feature/a.rs";
const RULES: &str = concat!(
    include_str!("../src/core/risk_default.toml"),
    "\n[[rule]]\nid = \"R-feature\"\nwhy = \"x\"\npaths = [\"src/feature/**\"]\n"
);
const WAITING: &str = "high risk, owner approval needed";
const HALTED: &str = "high risk, quorum (D-022): bosun is halted, no signature counts:";
const UNREADABLE: &str =
    "high risk, quorum (D-022): cannot tell whether bosun is halted, no signature counts:";
const QUORUM: &str =
    "high risk, quorum (D-022): 2 clean verdicts and orchestrator signature needed";

struct Key(Ed25519KeyPair, String);

fn key() -> Key {
    let rng = ring::rand::SystemRandom::new();
    let doc = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let pem = format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        b64::encode(doc.as_ref())
    );
    Key(Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap(), pem)
}

fn config(policy: &str) -> String {
    format!("project = \"widget\"\n\n[approvals]\npolicy = \"{policy}\"\n")
}

fn verdict_of(session: &str, head: &str, major: u32, approved: bool) -> String {
    let r = json!({"bosun": "verdict", "session": session, "head_sha": head,
        "approved": approved, "confidence": 0.95, "major": major});
    format!("Review verdict: approve 0.95, 0 blocker.\n\n{r}")
}

/// PR 1 (Closes #7, leased by `session_w`, authored by `octocat`) at HEAD changing `file`;
/// the base policy is `policy`.
fn pr(k: &Key, file: &str, policy: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "Closes #7");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    fake.issue(7, "task", &[], "task\n<!-- bosun:task {} -->");
    fake.comment_by(
        7,
        "octocat",
        &lease_note("session_w", "2026-12-06T12:00:00Z", "src/**"),
    );
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    let public = b64::encode(k.0.public_key().as_ref());
    let keys = format!("[[key]]\nid = \"k1\"\ned25519 = \"{public}\"\n");
    fake.content(".bosun/approvers.toml", MAIN, &keys);
    fake.content(".bosun/bosun.toml", BASE, &config(policy));
    fake.content(".bosun/risk.toml", BASE, RULES);
    fake.content(".bosun/bosun.toml", MAIN, &config(policy));
    fake.pull_file(1, file, "modified", 1, 0, Some(PLAIN));
    fake
}

fn quorum(k: &Key) -> FakeGithub {
    pr(k, GATE, "quorum")
}

fn post_sig(fake: &FakeGithub, k: &Key) {
    let msg = format!("bosun-autonomous v1 acme/widget#1 {HEAD}");
    let s = b64::encode(k.0.sign(msg.as_bytes()).as_ref());
    let body = format!("/bosun approve {HEAD} autonomous\nbosun-sig ed25519 k1 {s}");
    fake.comment_by(1, "claude", &body);
    fake.last_comment(|c| c.via_app = Some("claude".into()));
}

fn post_verdict(fake: &FakeGithub, session: &str, head: &str) {
    fake.comment_by(
        1,
        "github-actions[bot]",
        &verdict_of(session, head, 0, true),
    );
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

/// Signature plus the verdicts of `sessions` on `fake`.
fn with(fake: &FakeGithub, k: &Key, sessions: &[&str]) -> (i32, Value) {
    for s in sessions {
        post_verdict(fake, s, HEAD);
    }
    post_sig(fake, k);
    check(fake)
}

fn owner_path(fake: &FakeGithub, v: &Value, what: &str) {
    assert!(description(fake).starts_with(WAITING), "{what}: {v}");
}

#[test]
fn two_verdicts_pass_a_critical_floor_pr_with_workflows_and_weaken() {
    let k = key();
    let fake = quorum(&k);
    // The critical floor (workflows, `src/**`), a CI workflow and the weaken capability.
    fake.pull_file(1, ".github/workflows/x.yml", "modified", 1, 0, Some(PLAIN));
    fake.pull_file(1, "src/core/meta.rs", "modified", 1, 0, Some(PLAIN));
    fake.label(1, "bosun:capability/weaken");
    let (code, v) = with(&fake, &k, &["session_r", "session_s"]);
    assert_eq!(code, 0, "{v}");
    let by = v["data"]["approved_by"].as_str().unwrap();
    assert!(
        by.starts_with("key k1 (") && by.contains("quorum, D-022"),
        "{by}"
    );
    assert_eq!(v["data"]["policy"], "quorum");
    assert_eq!(v["data"]["policy_source"], "explicit");
    assert_eq!(
        v["data"]["quorum_verdicts"],
        json!(["session_r", "session_s"])
    );
}

#[test]
fn one_verdict_or_none_does_not_pass_and_the_wait_is_not_an_owner_wait() {
    for sessions in [&["session_r"][..], &[][..]] {
        let k = key();
        let fake = quorum(&k);
        let (code, v) = with(&fake, &k, sessions);
        assert_eq!(code, 5, "{v}");
        let d = description(&fake);
        assert!(d.starts_with(QUORUM) && !d.starts_with(WAITING), "{d}");
        assert_eq!(v["data"]["policy"], "quorum");
    }
}

#[test]
fn a_signature_alone_does_not_pass_even_with_two_verdicts_missing() {
    let k = key();
    let fake = quorum(&k);
    post_verdict(&fake, "session_r", HEAD);
    post_verdict(&fake, "session_s", HEAD);
    // No signature: the verdicts alone do not approve a high-risk PR of this class.
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
}

#[test]
fn two_verdicts_from_the_same_session_count_once() {
    let k = key();
    let fake = quorum(&k);
    let (code, v) = with(&fake, &k, &["session_r", "session_r"]);
    assert_eq!(code, 5, "{v}");
    assert!(description(&fake).starts_with(QUORUM));
}

#[test]
fn a_verdict_from_the_author_or_a_lease_holder_does_not_count() {
    let k = key();
    let fake = quorum(&k);
    let (code, v) = with(&fake, &k, &["session_w", "session_r"]);
    assert_eq!(code, 5, "{v}");
    assert!(description(&fake).starts_with(QUORUM));
    // A leaseholder of another linked issue is excluded as well.
    let fake = quorum(&k);
    fake.issue(8, "other", &[], "x");
    fake.pull_set(1, "body", json!("Closes #7\nCloses #8"));
    fake.comment(
        8,
        &lease_note("session_s", "2026-12-06T12:00:00Z", "src/**"),
    );
    let (code, v) = with(&fake, &k, &["session_r", "session_s"]);
    assert_eq!(code, 5, "{v}");
}

#[test]
fn a_request_for_changes_on_the_head_blocks_even_with_two_clean_verdicts() {
    for before in [true, false] {
        let k = key();
        let fake = quorum(&k);
        let reject = |f: &FakeGithub| {
            f.comment_by(
                1,
                "github-actions[bot]",
                &verdict_of("session_x", HEAD, 1, false),
            )
        };
        if before {
            reject(&fake);
        }
        post_verdict(&fake, "session_r", HEAD);
        post_verdict(&fake, "session_s", HEAD);
        if !before {
            reject(&fake);
        }
        post_sig(&fake, &k);
        let (code, v) = check(&fake);
        assert_eq!(code, 5, "{before}: {v}");
        assert!(description(&fake).starts_with(QUORUM), "{before}");
        assert_eq!(sign(&fake, &k, "session_o").0, 1, "{before}");
    }
}

#[test]
fn verdicts_on_another_head_or_with_a_major_finding_do_not_count() {
    let k = key();
    let fake = quorum(&k);
    post_verdict(&fake, "session_r", HEAD);
    post_verdict(&fake, "session_s", OTHER);
    post_sig(&fake, &k);
    assert_eq!(check(&fake).0, 5);
    let fake = quorum(&k);
    post_verdict(&fake, "session_r", HEAD);
    fake.comment_by(
        1,
        "github-actions[bot]",
        &verdict_of("session_s", HEAD, 1, true),
    );
    post_sig(&fake, &k);
    assert_eq!(check(&fake).0, 5);
}

#[test]
fn the_trust_root_stays_with_the_owner_also_by_rename_and_case() {
    for (file, from) in [
        (".bosun/approvers.toml", None),
        (".bosun/bosun.toml", None),
        (".BOSUN/Approvers.toml", None),
        ("docs/moved.txt", Some(".bosun/approvers.toml")),
        ("docs/moved.txt", Some(".bosun/bosun.toml")),
        ("src/../x.rs", None),
    ] {
        let k = key();
        let fake = quorum(&k);
        let status = if from.is_some() {
            "renamed"
        } else {
            "modified"
        };
        fake.pull_file(1, file, status, 1, 0, Some(PLAIN));
        if let Some(f) = from {
            fake.renamed_from(1, f);
        }
        let (code, v) = with(&fake, &k, &["session_r", "session_s"]);
        assert_eq!(code, 5, "{file} {from:?}: {v}");
        owner_path(&fake, &v, file);
        assert_eq!(sign(&fake, &k, "session_o").0, 1, "{file} {from:?}");
    }
}

#[test]
fn a_halted_bosun_counts_no_signature() {
    let k = key();
    let fake = quorum(&k);
    fake.variable("BOSUN_HALT", "1");
    let (code, v) = with(&fake, &k, &["session_r", "session_s"]);
    assert_eq!(code, 5, "{v}");
    assert!(
        description(&fake).starts_with(HALTED),
        "{}",
        description(&fake)
    );
    assert!(!description(&fake).starts_with(WAITING));
    assert_eq!(sign(&fake, &k, "session_o").0, 4);
}

#[test]
fn an_unreadable_halt_state_counts_no_signature() {
    let k = key();
    let fake = quorum(&k);
    fake.fail_path("/issues", 500);
    let (code, v) = with(&fake, &k, &["session_r", "session_s"]);
    assert_eq!(code, 5, "{v}");
    assert!(
        description(&fake).starts_with(UNREADABLE),
        "{}",
        description(&fake)
    );
    assert!(!description(&fake).starts_with(WAITING));
}

#[test]
fn the_policy_is_read_from_the_base_never_the_head() {
    let k = key();
    // The head sets quorum, the base says owner: nothing counts.
    let fake = pr(&k, GATE, "owner");
    fake.content(".bosun/bosun.toml", HEAD, &config("quorum"));
    let (code, v) = with(&fake, &k, &["session_r", "session_s"]);
    assert_eq!(code, 5, "{v}");
    assert!(description(&fake).starts_with(WAITING));
    assert!(v["data"].get("policy").is_none());
    // And a PR that sets it in `.bosun/bosun.toml` is the trust root anyway.
    let fake = pr(&k, ".bosun/bosun.toml", "quorum");
    fake.content(".bosun/bosun.toml", HEAD, &config("quorum"));
    let (code, _) = with(&fake, &k, &["session_r", "session_s"]);
    assert_eq!(code, 5);
}

#[test]
fn in_bosun_the_judging_code_is_owner_only_and_elsewhere_it_is_not() {
    for file in [
        "src/core/risk.rs",
        "src/core/approve.rs",
        "src/cli/risk_check.rs",
    ] {
        let k = key();
        let fake = pr(&k, file, "quorum");
        fake.content("src/cli/risk_check.rs", BASE, "// marker");
        let (code, v) = with(&fake, &k, &["session_r", "session_s"]);
        assert_eq!(code, 5, "{file}: {v}");
        owner_path(&fake, &v, file);
        assert_eq!(sign(&fake, &k, "session_o").0, 1, "{file}");
        // Without the marker at base it is some other repo's file.
        let fake = pr(&k, file, "quorum");
        let (code, v) = with(&fake, &k, &["session_r", "session_s"]);
        assert_eq!(code, 0, "{file}: {v}");
    }
}

#[test]
fn a_secrets_match_counts_with_the_quorum() {
    let k = key();
    let fake = quorum(&k);
    fake.pull_file(
        1,
        "src/a.env",
        "modified",
        1,
        0,
        Some("@@ -0,0 +1,1 @@\n+API_KEY = abc"),
    );
    let (code, v) = with(&fake, &k, &["session_r", "session_s"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn an_unlinked_pr_or_an_unleased_task_gets_no_quorum() {
    let k = key();
    let fake = quorum(&k);
    fake.pull_set(1, "body", json!(""));
    let (code, v) = with(&fake, &k, &["session_r", "session_s"]);
    assert_eq!(code, 5, "{v}");
    assert!(description(&fake).starts_with(WAITING));
}

#[test]
fn a_mode_autonomous_preset_is_a_quorum_and_booster_stays_autonomous() {
    for (mode, want) in [("autonomous", "quorum"), ("booster", "autonomous")] {
        let k = key();
        let fake = pr(&k, GATE, "owner");
        let c = format!("project = \"widget\"\n[mode]\nname = \"{mode}\"\n");
        fake.content(".bosun/bosun.toml", BASE, &c);
        let (code, v) = with(&fake, &k, &["session_r", "session_s"]);
        assert_eq!(code, 0, "{mode}: {v}");
        assert_eq!(v["data"]["policy"], want);
        assert_eq!(v["data"]["policy_source"], "preset");
    }
}

fn sign(fake: &FakeGithub, k: &Key, session: &str) -> (i32, Value) {
    let dir = tempfile::tempdir().unwrap().keep();
    let path = dir.join("key.pem");
    std::fs::write(&path, &k.1).unwrap();
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", NOW)
        .env("BOSUN_SESSION", session)
        .env("CCR_AGENT_PROXY_ENABLED", "true")
        .args(["approve", &format!("1@{HEAD}"), "--bootstrap", "--print"])
        .arg("--sign-key")
        .arg(&path)
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

#[test]
fn approve_signs_the_autonomous_form_after_two_clean_verdicts() {
    let k = key();
    let fake = quorum(&k);
    fake.label(1, "bosun:capability/weaken");
    fake.pull_file(1, ".github/workflows/x.yml", "modified", 1, 0, Some(PLAIN));
    post_verdict(&fake, "session_r", HEAD);
    post_verdict(&fake, "session_s", HEAD);
    let (code, v) = sign(&fake, &k, "session_o");
    assert_eq!(code, 0, "{v}");
    let body = v["data"]["body"].as_str().unwrap();
    assert!(
        body.contains("Signed under the quorum policy (D-022), not an owner answer"),
        "{body}"
    );
    assert!(body.starts_with(&format!("/bosun approve {HEAD} autonomous\n")));
    assert_eq!(v["data"]["policy"], "quorum");
}

#[test]
fn approve_refuses_with_fewer_than_two_counting_verdicts() {
    let k = key();
    // None, one, the same session twice, the author's, the signer's own (not counted).
    for sessions in [
        &[][..],
        &["session_r"][..],
        &["session_r", "session_r"][..],
        &["session_r", "session_w"][..],
        &["session_r", "session_o"][..],
    ] {
        let fake = quorum(&k);
        for s in sessions {
            post_verdict(&fake, s, HEAD);
        }
        let (code, v) = sign(&fake, &k, "session_o");
        assert_eq!(code, 1, "{sessions:?}: {v}");
    }
}

#[test]
fn approve_refuses_the_trust_root_and_a_halted_bosun() {
    let k = key();
    let fake = quorum(&k);
    fake.pull_file(1, ".bosun/approvers.toml", "modified", 1, 0, Some(PLAIN));
    post_verdict(&fake, "session_r", HEAD);
    post_verdict(&fake, "session_s", HEAD);
    assert_eq!(sign(&fake, &k, "session_o").0, 1);
    let fake = quorum(&k);
    post_verdict(&fake, "session_r", HEAD);
    post_verdict(&fake, "session_s", HEAD);
    fake.variable("BOSUN_HALT", "1");
    assert_eq!(sign(&fake, &k, "session_o").0, 4);
}

#[test]
fn wave_agents_of_one_session_count_once_and_not_as_the_author() {
    let k = key();
    // S/r1 and S/r2 are one session.
    let fake = quorum(&k);
    let (code, v) = with(&fake, &k, &["session_r/r1", "session_r/r2"]);
    assert_eq!(code, 5, "{v}");
    assert!(description(&fake).starts_with(QUORUM));
    // Two real sessions still pass.
    let fake = quorum(&k);
    let (code, v) = with(&fake, &k, &["session_r/r1", "session_s/r1"]);
    assert_eq!(code, 0, "{v}");
    // The author (lease holder `session_w`) has wave agents too: they do not count.
    let fake = quorum(&k);
    let (code, v) = with(&fake, &k, &["session_w/x", "session_r"]);
    assert_eq!(code, 5, "{v}");
    assert!(description(&fake).starts_with(QUORUM));
}

#[test]
fn a_relayed_verdict_whose_relayer_holds_a_lease_does_not_count() {
    let k = key();
    let fake = quorum(&k);
    post_verdict(&fake, "session_r", HEAD);
    let relayed = verdict_of("session_s", HEAD, 0, true)
        .replace("\"head_sha\"", "\"relayed_by\":\"session_w\",\"head_sha\"");
    fake.comment_by(1, "github-actions[bot]", &relayed);
    post_sig(&fake, &k);
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    assert!(description(&fake).starts_with(QUORUM));
}

#[test]
fn an_edited_verdict_comment_does_not_count() {
    let k = key();
    let fake = quorum(&k);
    post_verdict(&fake, "session_r", HEAD);
    fake.comment_edited(
        1,
        "github-actions[bot]",
        &verdict_of("session_s", HEAD, 0, true),
    );
    post_sig(&fake, &k);
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
}

#[test]
fn an_unseen_diff_gets_no_quorum_signature() {
    let k = key();
    let fake = quorum(&k);
    fake.pull_set(1, "changed_files", json!(5));
    let (code, v) = with(&fake, &k, &["session_r", "session_s"]);
    assert_eq!(code, 5, "{v}");
    owner_path(&fake, &v, "unseen");
    // And `approve` refuses to sign it.
    let fake = quorum(&k);
    fake.pull_set(1, "changed_files", json!(5));
    post_verdict(&fake, "session_r", HEAD);
    post_verdict(&fake, "session_s", HEAD);
    assert_eq!(sign(&fake, &k, "session_o").0, 1);
}

#[test]
fn trust_root_covers_the_risk_workflow_anywhere_and_in_bosun_the_judging_files() {
    for file in [".github/workflows/risk.yml", ".GitHub/Workflows/Risk.yml"] {
        let k = key();
        let fake = pr(&k, file, "quorum");
        let (code, v) = with(&fake, &k, &["session_r", "session_s"]);
        assert_eq!(code, 5, "{file}: {v}");
        assert_eq!(sign(&fake, &k, "session_o").0, 1, "{file}");
    }
}

#[test]
fn approve_does_not_count_the_signers_wave_agents_as_reviewers() {
    let k = key();
    let fake = quorum(&k);
    post_verdict(&fake, "session_r", HEAD);
    post_verdict(&fake, "session_o/x", HEAD);
    assert_eq!(sign(&fake, &k, "session_o").0, 1);
}
