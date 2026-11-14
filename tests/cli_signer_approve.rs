//! The keyless fast lane (D-017): a `signer` key signs only the autonomous form, `approve --as
//! signer` needs a green `ci-ok`, `--via-signer` returns the dispatch call, and risk-check says
//! when the PR author is the owner.
mod common;

use bosun::github::b64;
use common::fake::FakeGithub;
use common::{bosun, lease_note, one_object, run};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};

const BASE: &str = "base0000";
const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
const MAIN: &str = "refs/heads/main";
const NOW: &str = "2026-12-05T10:00:00Z";
/// The default rules plus a repo rule that makes `src/feature/**` high risk.
const RULES: &str = concat!(
    include_str!("../src/core/risk_default.toml"),
    "\n[[rule]]\nid = \"R-feature\"\nwhy = \"x\"\npaths = [\"src/feature/**\"]\n"
);
const PLAIN: &str = "@@ -0,0 +1,1 @@\n+x";

struct Key(Ed25519KeyPair, String);

fn key() -> Key {
    let rng = ring::rand::SystemRandom::new();
    let doc = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    // The PEM label is split so this test file adds no line that looks like a private key.
    let label = concat!("PRIVATE", " KEY");
    let pem = format!(
        "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
        b64::encode(doc.as_ref())
    );
    Key(Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap(), pem)
}

/// The default branch's `.bosun/bosun.toml`: the signer opt-in and the gate App's checks mode.
fn config(policy: &str, signer: bool, checks: bool) -> String {
    let mut c = format!("project = \"widget\"\n\n[approvals]\npolicy = \"{policy}\"\n");
    if signer {
        c.push_str("signer = true\n");
    }
    if checks {
        c.push_str("\n[checks]\nmode = \"checks\"\napp_id = 4242\napp_slug = \"gate-app\"\n");
    }
    c
}

/// A checkout directory whose config is the default branch's, as the signer workflow has it.
fn checkout(checks: bool) -> std::path::PathBuf {
    let dir = tempfile::tempdir().unwrap().keep();
    std::fs::create_dir_all(dir.join(".bosun")).unwrap();
    std::fs::write(
        dir.join(".bosun/bosun.toml"),
        config("autonomous", true, checks),
    )
    .unwrap();
    dir
}

/// The gate App's green `ci-ok` run on HEAD.
fn ci_ok(fake: &FakeGithub) {
    fake.app_run(HEAD, "ci-ok", "success", "ok", "gate-app", 4242);
}

/// PR 1 by `author` (Closes #7, leased by `session_w`) at HEAD changing src/a.rs; owner `ada`;
/// key `k1` enrolled as `role`.
fn pr(k: &Key, role: &str, policy: &str, author: &str) -> FakeGithub {
    pr_v(k, role, policy, author, Some(("session_r", 0)))
}

/// As `pr`, with one verdict (`session`, `major`) on the head, or none.
fn pr_v(
    k: &Key,
    role: &str,
    policy: &str,
    author: &str,
    verdict: Option<(&str, u32)>,
) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "Closes #7");
    fake.pull_as_issue(1, "change");
    fake.pull_set(1, "head", json!({"sha": HEAD, "ref": "feature"}));
    fake.pull_set(1, "user", json!({"login": author}));
    fake.issue(7, "task", &[], "task\n<!-- bosun:task {} -->");
    fake.comment_by(
        7,
        author,
        &lease_note("session_w", "2026-12-06T12:00:00Z", "src/**"),
    );
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    let public = b64::encode(k.0.public_key().as_ref());
    let keys = format!("[[key]]\nid = \"k1\"\ned25519 = \"{public}\"\nrole = \"{role}\"\n");
    fake.content(".bosun/approvers.toml", MAIN, &keys);
    fake.content(".bosun/risk.toml", BASE, RULES);
    let cfg = config(policy, true, true);
    fake.content(".bosun/bosun.toml", BASE, &cfg);
    fake.content(".bosun/bosun.toml", MAIN, &cfg);
    fake.pull_file(1, "src/feature/a.rs", "modified", 1, 0, Some(PLAIN));
    if let Some((session, major)) = verdict {
        let r = json!({"bosun": "verdict", "session": session, "head_sha": HEAD,
            "approved": true, "confidence": 0.95, "major": major});
        // The gate App's comment counts in checks mode, GitHub Actions' otherwise.
        for bot in ["github-actions[bot]", "gate-app[bot]"] {
            fake.comment_by(1, bot, &format!("Review verdict: approve 0.95.\n\n{r}"));
        }
    }
    fake
}

fn bosun_run(fake: &FakeGithub, args: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", NOW)
        .env("BOSUN_SESSION", "signer-ci-1")
        .args(args)
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

fn sign(fake: &FakeGithub, k: &Key, extra: &[&str]) -> (i32, Value) {
    sign_at(fake, k, extra, None)
}

/// As `sign`, from a checkout directory `at` (its .bosun/bosun.toml sets `[checks]`).
fn sign_at(
    fake: &FakeGithub,
    k: &Key,
    extra: &[&str],
    at: Option<&std::path::Path>,
) -> (i32, Value) {
    let path = tempfile::tempdir().unwrap().keep().join("key.pem");
    std::fs::write(&path, &k.1).unwrap();
    let target = format!("1@{HEAD}");
    let mut args = vec!["approve", &target, "--sign-key", path.to_str().unwrap()];
    args.extend_from_slice(extra);
    let default = (at.is_none()).then(|| checkout(true));
    let at = at.or(default.as_deref());
    let at = at.map(|d| d.to_str().unwrap().to_string());
    let mut full = Vec::new();
    if let Some(d) = &at {
        full.extend(["-C", d.as_str()]);
    }
    full.extend(args);
    bosun_run(fake, &full)
}

const AS_SIGNER: [&str; 5] = ["--bootstrap", "--as", "signer", "--requester", "session_q"];

#[test]
fn the_signer_signs_the_autonomous_form_once_ci_ok_is_green() {
    let k = key();
    let fake = pr(&k, "signer", "autonomous", "octocat");
    ci_ok(&fake);
    let (code, v) = sign(&fake, &k, &AS_SIGNER);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["approved"][0]["policy"], "autonomous");
    let comments = fake.comments_of(1);
    let body = &comments.last().unwrap().body;
    assert!(
        body.starts_with(&format!("/bosun approve {HEAD} autonomous\n")),
        "{body}"
    );
    // risk-check counts it: a signer key stands in for an owner key in the autonomous form.
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn only_the_gate_apps_newest_ci_ok_run_counts() {
    let k = key();
    // A ci-ok status never counts, however green.
    let fake = pr(&k, "signer", "autonomous", "octocat");
    fake.status(HEAD, "ci-ok", "success", "ok");
    assert_eq!(sign(&fake, &k, &AS_SIGNER).0, 1, "a status");
    for (what, runs, want) in [
        ("missing", vec![], 1),
        (
            "failure",
            vec![("gate-app", "completed", Some("failure"))],
            1,
        ),
        ("running", vec![("gate-app", "in_progress", None)], 1),
        (
            "older failing, newer passing",
            vec![
                ("gate-app", "completed", Some("failure")),
                ("gate-app", "completed", Some("success")),
            ],
            0,
        ),
        (
            "newer failing",
            vec![
                ("gate-app", "completed", Some("success")),
                ("gate-app", "completed", Some("failure")),
            ],
            1,
        ),
        (
            "another app",
            vec![("evil-app", "completed", Some("success"))],
            1,
        ),
        (
            "github-actions (a push workflow can post it)",
            vec![("github-actions", "completed", Some("success"))],
            1,
        ),
    ] {
        let fake = pr(&k, "signer", "autonomous", "octocat");
        for (app, st, c) in &runs {
            let id = match *app {
                "gate-app" => 4242,
                "github-actions" => 15368,
                _ => 7,
            };
            fake.check_run_app(HEAD, "ci-ok", st, *c, app, id);
        }
        let before = fake.comments_of(1).len();
        let (code, v) = sign(&fake, &k, &AS_SIGNER);
        assert_eq!(code, want, "{what}: {v}");
        if want == 1 {
            assert!(v["data"]["ci_ok"].is_string(), "{what}: {v}");
            assert_eq!(fake.comments_of(1).len(), before, "{what}: nothing signed");
        }
    }
}

#[test]
fn the_requester_is_never_the_independent_reviewer() {
    let k = key();
    let fake = pr_v(
        &k,
        "signer",
        "autonomous",
        "octocat",
        Some(("session_q", 0)),
    );
    ci_ok(&fake);
    let before = fake.comments_of(1).len();
    let (code, v) = sign(&fake, &k, &AS_SIGNER);
    assert_eq!(code, 1, "{v}");
    assert_eq!(fake.comments_of(1).len(), before, "nothing signed");
    // Another reviewer's clean verdict signs; --requester is required with --as signer only.
    let fake = pr(&k, "signer", "autonomous", "octocat");
    ci_ok(&fake);
    assert_eq!(sign(&fake, &k, &["--bootstrap", "--as", "signer"]).0, 2);
    assert_eq!(sign(&fake, &k, &["--bootstrap", "--requester", "x y"]).0, 2);
    assert_eq!(sign(&fake, &k, &AS_SIGNER).0, 0);
}

#[test]
fn the_signer_role_needs_bootstrap_and_the_autonomous_policy() {
    let k = key();
    let fake = pr(&k, "signer", "autonomous", "octocat");
    ci_ok(&fake);
    let no_boot = ["--as", "signer", "--requester", "session_q"];
    assert_eq!(sign(&fake, &k, &no_boot).0, 2, "no --bootstrap");
    // Without `--as signer` the enrolled role does not match.
    assert_eq!(sign(&fake, &k, &["--bootstrap"]).0, 1);
    for policy in ["owner-gated", "orchestrator", "owner"] {
        let fake = pr(&k, "signer", policy, "octocat");
        ci_ok(&fake);
        let (code, v) = sign(&fake, &k, &AS_SIGNER);
        assert_eq!(code, 1, "{policy}: {v}");
        assert!(v["error"]["fix"].as_str().unwrap().contains("autonomous"));
    }
    // An owner key never signs as signer.
    let o = key();
    let fake = pr(&o, "owner", "autonomous", "octocat");
    ci_ok(&fake);
    assert_eq!(sign(&fake, &o, &AS_SIGNER).0, 1);
    // The usual autonomous checks still hold: a verdict with a major finding signs nothing.
    let fake = pr_v(
        &k,
        "signer",
        "autonomous",
        "octocat",
        Some(("session_r", 2)),
    );
    ci_ok(&fake);
    let before = fake.comments_of(1).len();
    assert_eq!(sign(&fake, &k, &AS_SIGNER).0, 1, "major finding");
    assert_eq!(fake.comments_of(1).len(), before);
    let bad = pr(&k, "signer", "autonomous", "octocat");
    ci_ok(&bad);
    bad.pull_file(1, ".github/workflows/ci.yml", "modified", 1, 0, Some(PLAIN));
    assert_eq!(sign(&bad, &k, &AS_SIGNER).0, 1, "excluded path");
}

/// Post a comment signed by key k1 over `msg` with the first line `first`.
fn post(fake: &FakeGithub, k: &Key, first: &str, msg: &str) {
    let s = b64::encode(k.0.sign(msg.as_bytes()).as_ref());
    fake.comment_by(1, "claude", &format!("{first}\nbosun-sig ed25519 k1 {s}"));
}

#[test]
fn a_signer_key_counts_for_nothing_but_the_autonomous_form() {
    let k = key();
    let early = "2026-11-20T10:00:00Z";
    let m = |prefix: &str| format!("{prefix} v1 acme/widget#1 {HEAD}");
    let v2 = format!("bosun-approve v2 acme/widget#1 {HEAD} after-review 42");
    let cases = [
        // (policy, first line, the form's own message)
        (
            "owner-gated",
            format!("/bosun approve {HEAD}"),
            m("bosun-approve"),
        ),
        (
            "owner-gated",
            format!("/bosun approve {HEAD} after-review 42"),
            v2,
        ),
        (
            "orchestrator",
            format!("/bosun approve {HEAD} bootstrap"),
            m("bosun-bootstrap"),
        ),
        (
            "owner-gated",
            format!("/bosun approve {HEAD} autonomous"),
            m("bosun-autonomous"),
        ),
    ];
    for (policy, first, msg) in cases {
        let boot = |fake: &FakeGithub| {
            fake.content(
                ".bosun/bosun.toml",
                BASE,
                &format!("project = \"widget\"\n\n[approvals]\npolicy = \"{policy}\"\nbootstrap_until = \"2026-11-30\"\n"),
            );
        };
        // The signer key is rejected by the role check alone...
        let fake = pr(&k, "signer", policy, "octocat");
        boot(&fake);
        post(&fake, &k, &first, &msg);
        let (code, v) = run(&fake, "session_test", early, &["risk-check", "1"]);
        assert_eq!(code, 5, "{first}: {v}");
        assert!(v["data"]["approved_by"].is_null(), "{first}: {v}");
    }
    // ...and the same signatures by an owner key count (the control), where the form applies.
    for (policy, first, msg, counts) in [
        (
            "orchestrator",
            format!("/bosun approve {HEAD} bootstrap"),
            m("bosun-bootstrap"),
            true,
        ),
        (
            "owner-gated",
            format!("/bosun approve {HEAD}"),
            m("bosun-approve"),
            true,
        ),
    ] {
        let o = key();
        let fake = pr(&o, "owner", policy, "octocat");
        fake.content(
            ".bosun/bosun.toml",
            BASE,
            &format!("project = \"widget\"\n\n[approvals]\npolicy = \"{policy}\"\nbootstrap_until = \"2026-11-30\"\n"),
        );
        post(&fake, &o, &first, &msg);
        let (code, v) = run(&fake, "session_test", early, &["risk-check", "1"]);
        assert_eq!(code == 0, counts, "control {first}: {v}");
    }
}

#[test]
fn via_signer_returns_the_dispatch_call_and_posts_nothing() {
    let k = key();
    let fake = pr(&k, "signer", "autonomous", "octocat");
    let target = format!("1@{HEAD}");
    let (code, v) = bosun_run(&fake, &["approve", &target, "--via-signer"]);
    assert_eq!(code, 0, "{v}");
    let next = &v["next"][0];
    assert_eq!(next["tool"], "mcp__github__actions_run_trigger");
    assert_eq!(
        next["args"],
        json!({"method": "run_workflow", "owner": "acme", "repo": "widget",
            "workflow_id": "signer.yml", "ref": "main",
            "inputs": {"pr": "1", "head": HEAD, "requester": "signer-ci-1"}})
    );
    assert!(
        fake.comments_of(1)
            .iter()
            .all(|c| !c.body.contains("bosun-sig"))
    );
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SESSION", "not a plain id!")
        .args(["approve", &target, "--via-signer"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "{:?}", one_object(&out.stdout));
    let stale = format!("1@{}", "f".repeat(40));
    assert_eq!(bosun_run(&fake, &["approve", &stale, "--via-signer"]).0, 3);
    for extra in ["--list", "--print"] {
        assert_eq!(
            bosun_run(&fake, &["approve", &target, "--via-signer", extra]).0,
            2,
            "{extra}"
        );
    }
    assert_eq!(
        bosun_run(
            &fake,
            &["approve", &target, "--via-signer", "--sign-key", "k.pem"]
        )
        .0,
        2
    );
    fake.variable("BOSUN_HALT", "1");
    assert_eq!(bosun_run(&fake, &["approve", &target, "--via-signer"]).0, 4);
}

#[test]
fn risk_check_says_when_the_author_is_the_owner() {
    let k = key();
    // Under the autonomous policy the fix names the CI signer; under any other it must not.
    for (policy, signer) in [
        ("autonomous", true),
        ("owner-gated", false),
        ("owner", false),
    ] {
        let fake = pr(&k, "owner", policy, "Ada");
        let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
        assert_eq!(code, 5, "{policy}: {v}");
        assert_eq!(v["data"]["author_is_owner"], true);
        let fix = v["error"]["fix"].as_str().unwrap();
        assert!(fix.contains("self-review"), "{fix}");
        assert_eq!(fix.contains("--via-signer"), signer, "{policy}: {fix}");
    }
    let fake = pr(&k, "owner", "owner-gated", "octocat");
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 5, "{v}");
    assert!(v["data"].get("author_is_owner").is_none());
}

#[test]
fn keygen_makes_and_enrolls_a_signer_key() {
    let dir = tempfile::tempdir().unwrap();
    let git = |a: &[&str]| {
        std::process::Command::new("git")
            .current_dir(dir.path())
            .args(a)
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    let out = bosun()
        .current_dir(dir.path())
        .args([
            "keygen",
            "--role",
            "signer",
            "--id",
            "signer-ci",
            "--enroll",
            "--out",
        ])
        .arg(dir.path().join("../signer-test.pem"))
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    assert_eq!(v["data"]["role"], "signer");
    let t = std::fs::read_to_string(dir.path().join(".bosun/approvers.toml")).unwrap();
    assert!(t.contains("role = \"signer\""), "{t}");
    let _ = std::fs::remove_file(dir.path().join("../signer-test.pem"));
}

fn doctor_item(dir: &std::path::Path, fake: &FakeGithub) -> Option<Value> {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("CLAUDE_CONFIG_DIR", dir.join(".test-claude"))
        .arg("-C")
        .arg(dir)
        .args(["doctor", "--full"])
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    v["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "CI signer")
        .cloned()
}

#[test]
fn doctor_checks_the_signer_workflow_and_key_under_the_autonomous_policy() {
    let write = |d: &std::path::Path, rel: &str, t: &str| {
        let p = d.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    };
    let fake = FakeGithub::start();
    // Not autonomous: no such item.
    let d = tempfile::tempdir().unwrap();
    write(d.path(), ".bosun/bosun.toml", "project = \"w\"\n");
    assert!(doctor_item(d.path(), &fake).is_none());
    // Autonomous without `[approvals] signer = true`: off by default, no setup warnings.
    let d = tempfile::tempdir().unwrap();
    write(
        d.path(),
        ".bosun/bosun.toml",
        "project = \"w\"\n\n[mode]\nname = \"autonomous\"\n",
    );
    let it = doctor_item(d.path(), &fake).expect("item");
    assert_eq!(it["status"], "skipped", "{it}");
    assert!(
        it["detail"].as_str().unwrap().contains("off (default)"),
        "{it}"
    );
    // Autonomous (via the mode preset), nothing set up: a warning with the owner step.
    let d = tempfile::tempdir().unwrap();
    write(
        d.path(),
        ".bosun/bosun.toml",
        "project = \"w\"\n\n[mode]\nname = \"autonomous\"\n\n[approvals]\nsigner = true\n",
    );
    let it = doctor_item(d.path(), &fake).expect("item");
    assert_eq!(it["status"], "warn", "{it}");
    assert!(it["fix"].as_str().unwrap().contains("keygen --role signer"));
    assert!(it["detail"].as_str().unwrap().contains("cannot see"));
    // An owner key does not satisfy it: signer.yml runs `--as signer`.
    write(
        d.path(),
        ".github/workflows/signer.yml",
        "on: workflow_dispatch\n",
    );
    write(
        d.path(),
        ".bosun/approvers.toml",
        "[[key]]\nid = \"o\"\ned25519 = \"AAAA\"\n",
    );
    assert_eq!(doctor_item(d.path(), &fake).unwrap()["status"], "warn");
    // Workflow and a signer key present: ok.
    write(
        d.path(),
        ".github/workflows/signer.yml",
        "on: workflow_dispatch\n",
    );
    write(
        d.path(),
        ".bosun/approvers.toml",
        "[[key]]\nid = \"s\"\ned25519 = \"AAAA\"\nrole = \"signer\"\n",
    );
    assert_eq!(doctor_item(d.path(), &fake).unwrap()["status"], "ok");
}

#[test]
fn the_signer_is_opt_in_and_needs_checks_mode() {
    let k = key();
    let cfg_at = |f: &FakeGithub, text: &str| {
        f.content(".bosun/bosun.toml", BASE, text);
        f.content(".bosun/bosun.toml", MAIN, text);
    };
    let (off, on_statuses) = (
        config("autonomous", false, true),
        config("autonomous", true, false),
    );
    let signer_false = off.replace(
        "policy = \"autonomous\"\n",
        "policy = \"autonomous\"\nsigner = false\n",
    );
    for (what, text, want) in [
        ("signer not set", off.clone(), "signer"),
        ("signer = false", signer_false, "signer"),
        ("statuses mode", on_statuses, "checks"),
    ] {
        let fake = pr(&k, "signer", "autonomous", "octocat");
        ci_ok(&fake);
        cfg_at(&fake, &text);
        let before = fake.comments_of(1).len();
        let (code, v) = sign(&fake, &k, &AS_SIGNER);
        assert_eq!(code, 1, "{what}: {v}");
        let fix = v["error"]["fix"].as_str().unwrap();
        assert!(fix.contains(want), "{what}: {fix}");
        assert_eq!(fake.comments_of(1).len(), before, "{what}: nothing signed");
    }
    // A checkout without the checks config cannot trust the gate App either.
    let fake = pr(&k, "signer", "autonomous", "octocat");
    ci_ok(&fake);
    assert_eq!(sign_at(&fake, &k, &AS_SIGNER, Some(&checkout(false))).0, 1);
    // On, in checks mode, with the gate App's green run: signs.
    let fake = pr(&k, "signer", "autonomous", "octocat");
    ci_ok(&fake);
    assert_eq!(sign(&fake, &k, &AS_SIGNER).0, 0);
}

#[test]
fn turning_the_signer_on_is_an_r_weaken_finding() {
    let k = key();
    let fake = pr(&k, "owner", "owner-gated", "octocat");
    let off = config("owner-gated", false, false);
    fake.content(".bosun/bosun.toml", BASE, &off);
    fake.pull_file(1, ".bosun/bosun.toml", "modified", 1, 0, Some(PLAIN));
    fake.content(
        ".bosun/bosun.toml",
        HEAD,
        &config("owner-gated", true, false),
    );
    let (code, v) = run(&fake, "session_test", NOW, &["risk-check", "1"]);
    assert_eq!(code, 5, "{v}");
    let m = v["data"]["matches"].to_string();
    assert!(
        m.contains("R-weaken") && m.contains("approvals.signer"),
        "{m}"
    );
}

#[test]
fn the_signer_refuses_what_risk_check_calls_a_weakening() {
    let k = key();
    let t = "#[test]\nfn a() { assert!(x); }\n";
    let weak = format!("{t}#[test]\n#[ignore]\nfn b() {{ assert!(y); }}\n");
    let fake = pr(&k, "signer", "autonomous", "octocat");
    ci_ok(&fake);
    fake.pull_file(1, "tests/a.rs", "modified", 3, 0, Some(PLAIN));
    fake.content("tests/a.rs", BASE, t);
    fake.content("tests/a.rs", HEAD, &weak);
    let before = fake.comments_of(1).len();
    let (code, v) = sign(&fake, &k, &AS_SIGNER);
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["message"].as_str().unwrap().contains("R-weaken"),
        "{v}"
    );
    assert_eq!(fake.comments_of(1).len(), before, "nothing signed");
    // The same file without the weakening signs.
    let fake = pr(&k, "signer", "autonomous", "octocat");
    ci_ok(&fake);
    fake.pull_file(1, "tests/a.rs", "modified", 3, 0, Some(PLAIN));
    fake.content("tests/a.rs", BASE, t);
    fake.content("tests/a.rs", HEAD, t);
    assert_eq!(sign(&fake, &k, &AS_SIGNER).0, 0);
    // A `weaken` capability label is the owner's too.
    let fake = pr(&k, "signer", "autonomous", "octocat");
    ci_ok(&fake);
    fake.pull_set(1, "labels", json!([{"name": "bosun:capability/weaken"}]));
    let (code, v) = sign(&fake, &k, &AS_SIGNER);
    assert_eq!(code, 1, "{v}");
}

fn doctor_remote(fake: &FakeGithub, remote: bool) -> (Value, Value) {
    let d = tempfile::tempdir().unwrap();
    let write = |rel: &str, t: &str| {
        let p = d.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    };
    write(
        ".bosun/bosun.toml",
        "project = \"w\"\n\n[mode]\nname = \"autonomous\"\n\n[approvals]\nsigner = true\n",
    );
    write(
        ".github/workflows/signer.yml",
        "on: workflow_dispatch\njobs:\n  sign:\n    environment: bosun-signer\n    steps:\n      - run: x\n",
    );
    let mut cmd = bosun();
    cmd.env("BOSUN_GITHUB_API", fake.url())
        .env("CLAUDE_CONFIG_DIR", d.path().join(".test-claude"))
        .arg("-C")
        .arg(d.path())
        .args(["doctor", "--full"]);
    if remote {
        cmd.arg("--remote");
    }
    let v = one_object(&cmd.output().unwrap().stdout);
    let items = v["data"]["items"].as_array().unwrap();
    let signer = items
        .iter()
        .find(|i| i["name"] == "CI signer")
        .cloned()
        .unwrap();
    let envs = v["data"]["environments"]
        .as_array()
        .and_then(|e| e.first().cloned());
    (signer, envs.unwrap_or(Value::Null))
}

#[test]
fn doctor_checks_the_signer_environment_is_limited_to_the_default_branch() {
    let custom = json!({"protected_branches": false, "custom_branch_policies": true});
    // ok: only the default branch.
    let fake = FakeGithub::start();
    fake.environment(
        "bosun-signer",
        custom.clone(),
        json!([{"name": "main", "type": "branch"}]),
    );
    let (_, env) = doctor_remote(&fake, true);
    assert_eq!(env["status"], "ok", "{env}");
    // A wrong branch, any branch or a missing environment warns.
    for (policy, branches, want) in [
        (
            custom.clone(),
            json!([{"name": "dev", "type": "branch"}]),
            "dev",
        ),
        (Value::Null, json!([]), "any branch"),
    ] {
        let fake = FakeGithub::start();
        fake.environment("bosun-signer", policy, branches);
        let (_, env) = doctor_remote(&fake, true);
        assert_eq!(env["status"], "warn", "{env}");
        assert!(env["detail"].as_str().unwrap().contains(want), "{env}");
    }
    // Unreadable (403): says what the owner verifies.
    let fake = FakeGithub::start();
    fake.fail_path("/environments/bosun-signer", 403);
    let (_, env) = doctor_remote(&fake, true);
    assert_eq!(env["status"], "warn", "{env}");
    assert!(
        env["detail"].as_str().unwrap().contains("verify: owner"),
        "{env}"
    );
    // Without --remote the signer item itself names the owner step.
    let (signer, _) = doctor_remote(&FakeGithub::start(), false);
    assert!(
        signer["detail"].as_str().unwrap().contains("verify: owner"),
        "{signer}"
    );
}
