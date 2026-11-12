//! `bosun metrics`: red-CI facts, the brake, cost, lead time, owner touches, blocks per rule.
mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object, run};
use serde_json::{Value, json};

const NOW: &str = "2026-11-12T12:00:00Z";
const ACTIONS: &str = "github-actions";

fn merged(fake: &FakeGithub, n: u64, at: &str) {
    fake.pull_merged(n, &format!("m{n}"));
    for k in ["merged_at", "closed_at", "updated_at"] {
        fake.pull_set(n, k, json!(at));
    }
}

/// A real red head: the `ci` job failed, so `ci-ok` failed.
fn red(fake: &FakeGithub, sha: &str) {
    fake.check_run_by(sha, "ci", "failure", ACTIONS);
    fake.check_run_by(sha, "ci-ok", "failure", ACTIONS);
}

fn pr(fake: &FakeGithub, n: u64, body: &str, heads: &[&str]) {
    fake.pull(n, &format!("PR {n}"), &format!("feat/{n}"), body);
    fake.pull_commit_shas(n, heads);
}

fn metrics(fake: &FakeGithub, args: &[&str]) -> Value {
    let mut a = vec!["metrics"];
    a.extend(args);
    let (code, v) = run(fake, "session_a", NOW, &a);
    assert_eq!(code, 0, "{v}");
    v["data"].clone()
}

/// Two merged PRs whose merge commits failed `ci-ok`.
fn two_red_mains(fake: &FakeGithub) {
    for n in [10, 11] {
        pr(fake, n, "", &[]);
        merged(fake, n, "2026-11-10T10:00:00Z");
        red(fake, &format!("m{n}"));
    }
}

#[test]
fn two_red_main_merges_trip_the_brake_and_no_main_red_issue_is_read() {
    let fake = FakeGithub::start();
    fake.issue(1, "main is red", &["main-red"], "");
    fake.close_issue(1, "2026-11-11T00:00:00Z");
    two_red_mains(&fake);
    let d = metrics(&fake, &[]);
    assert_eq!(d["brake"]["on"], true, "{d}");
    assert_eq!(d["brake"]["window"]["main_red"], 2);
    assert_eq!(d["brake"]["window"]["days"], 7);
    let why = d["brake"]["reasons"][0].as_str().unwrap();
    assert!(why.contains("main went red 2"), "{why}");
    for r in fake.requests() {
        assert!(!r.path.contains("main-red"), "{}", r.path);
        assert_eq!(r.method, "GET");
    }
}

#[test]
fn a_green_rerun_still_counts_and_other_sources_do_not() {
    let fake = FakeGithub::start();
    pr(&fake, 20, "", &["h20"]);
    red(&fake, "h20");
    fake.check_run_by("h20", "ci-ok", "success", ACTIONS);
    pr(&fake, 21, "", &["h21"]);
    fake.check_run_by("h21", "ci-ok", "failure", "some-other-app");
    pr(&fake, 22, "", &["h22"]);
    fake.status("h22", "ci-ok", "failure", "red");
    // A newer push cancelled the run, or a draft: `ci-ok` fails alone, no pushed head.
    pr(&fake, 23, "", &["h23a", "h23b"]);
    fake.check_run_by("h23a", "ci", "cancelled", ACTIONS);
    fake.check_run_by("h23a", "ci-ok", "failure", ACTIONS);
    fake.check_run_by("h23b", "ci-ok", "success", ACTIONS);
    pr(&fake, 25, "", &["h25"]);
    fake.pull_set(25, "draft", json!(true));
    fake.check_run_by("h25", "ci", "skipped", ACTIONS);
    fake.check_run_by("h25", "ci-ok", "failure", ACTIONS);
    // A hand-cancelled run on the last head of a ready PR stays red.
    pr(&fake, 26, "", &["h26"]);
    fake.check_run_by("h26", "ci", "cancelled", ACTIONS);
    fake.check_run_by("h26", "ci-ok", "failure", ACTIONS);
    let d = metrics(&fake, &[]);
    assert_eq!(
        d["red_ci"],
        json!({"failing_heads": 2, "pushed_heads": 3, "rate_pct": 66.7})
    );
}

fn revert_pr(fake: &FakeGithub, n: u64, user: &str) {
    let body = format!("Reverts #1.\n\n<!-- bosun:revert {n} -->");
    pr(fake, n, &body, &[]);
    fake.pull_set(n, "user", json!({"login": user}));
    merged(fake, n, "2026-11-11T10:00:00Z");
}

#[test]
fn only_a_writers_revert_marker_counts() {
    let fake = FakeGithub::start();
    fake.permission("reader", "read");
    revert_pr(&fake, 30, "reader");
    assert_eq!(metrics(&fake, &[])["brake"]["window"]["reverts"], 0);
    revert_pr(&fake, 31, "octocat");
    assert_eq!(metrics(&fake, &[])["brake"]["window"]["reverts"], 1);
    revert_pr(&fake, 32, "octocat");
    let d = metrics(&fake, &[]);
    assert_eq!(d["brake"]["window"]["reverts"], 2);
    assert_eq!(d["brake"]["on"], true);
}

#[test]
fn a_cancelled_main_run_still_counts_red() {
    let fake = FakeGithub::start();
    for n in [12, 13] {
        pr(&fake, n, "", &[]);
        merged(&fake, n, "2026-11-10T10:00:00Z");
        fake.check_run_by(&format!("m{n}"), "ci", "cancelled", ACTIONS);
        fake.check_run_by(&format!("m{n}"), "ci-ok", "failure", ACTIONS);
    }
    assert_eq!(metrics(&fake, &[])["brake"]["window"]["main_red"], 2);
}

#[test]
fn a_revert_branch_counts_after_its_marker_is_edited_away() {
    let fake = FakeGithub::start();
    for n in [33, 34] {
        fake.pull(
            n,
            "Revert",
            &format!("bosun/revert-{n}"),
            "body edited, marker gone",
        );
        merged(&fake, n, "2026-11-11T10:00:00Z");
    }
    let d = metrics(&fake, &[]);
    assert_eq!(d["brake"]["window"]["reverts"], 2, "{d}");
}

fn run_in(fake: &FakeGithub, mode: Option<&str>) -> Value {
    let dir = tempfile::tempdir().unwrap();
    if let Some(m) = mode {
        std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
        let toml = format!("[mode]\nname = \"{m}\"\n");
        std::fs::write(dir.path().join(".bosun/bosun.toml"), toml).unwrap();
    }
    let out = bosun()
        .current_dir(dir.path())
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", NOW)
        .arg("metrics")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    one_object(&out.stdout)["data"].clone()
}

#[test]
fn the_knob_follows_the_mode_and_a_red_rate_trips_steady_only() {
    let fake = FakeGithub::start();
    pr(&fake, 40, "", &["h40"]);
    red(&fake, "h40");
    pr(&fake, 41, "", &["h41"]);
    fake.check_run_by("h41", "ci-ok", "success", ACTIONS);
    let g = run_in(&fake, None);
    assert_eq!(g["mode"], "gatekeeper");
    assert_eq!(g["brake"]["knob"], "off");
    assert_eq!(g["brake"]["on"], false);
    assert_eq!(g["red_ci"]["rate_pct"], 50.0);
    let s = run_in(&fake, Some("steady"));
    assert_eq!(s["brake"]["knob"], "on");
    assert_eq!(s["brake"]["on"], true);
    let why = s["brake"]["reasons"][0].as_str().unwrap();
    assert!(why.contains("red-CI rate 1/2"), "{why}");
}

#[test]
fn a_gatekeeper_still_trips_on_main_red() {
    let fake = FakeGithub::start();
    two_red_mains(&fake);
    let g = run_in(&fake, None);
    assert_eq!(
        (g["brake"]["knob"].as_str(), g["brake"]["on"].as_bool()),
        (Some("off"), Some(true))
    );
}

#[test]
fn zero_days_is_usage() {
    let fake = FakeGithub::start();
    let (code, v) = run(&fake, "s", NOW, &["metrics", "--days", "0"]);
    assert_eq!(code, 2, "{v}");
}

const RULES: &str = "[[rule]]\nid = \"R-docs\"\nwhy = \"d\"\npaths = [\"docs/**\"]\n\n\
    [[rule]]\nid = \"R-api\"\nwhy = \"a\"\npaths = [\"api/**\"]\n\n\
    [[rule]]\nid = \"R-web\"\nwhy = \"w\"\npaths = [\"web/**\"]\n\n\
    [[rule]]\nid = \"R-gov\"\nwhy = \"g\"\npaths = [\".bosun/**\"]\n";

fn waiting(fake: &FakeGithub, sha: &str, ids: &str) {
    let d = format!("high risk, owner approval needed: {ids}");
    fake.status(sha, "risk-check", "failure", &d);
    fake.status(sha, "risk-check", "success", "approved by the owner");
}

#[test]
fn propose_counts_trusted_risk_check_rules_of_prs_merged_on_their_first_head() {
    let fake = FakeGithub::start();
    fake.content(".bosun/risk.toml", "main", RULES);
    // PRs 31-40 merged on their first head. Trusted risk-check: R-docs on all 10 (twice on
    // 40), R-api on 31-39, R-gov (floor) on all. Their bodies claim R-web: never trusted.
    for n in 31..=40u64 {
        let h = format!("h{n}");
        pr(&fake, n, &risk(0, &["R-web"]), &[&h]);
        fake.pull_head_sha(n, &h);
        merged(&fake, n, "2026-11-05T00:00:00Z");
        fake.check_run_by(&h, "ci-ok", "success", ACTIONS);
        let ids = if n == 40 {
            "R-docs, R-gov, R-docs"
        } else {
            "R-docs, R-api, R-gov"
        };
        waiting(&fake, &h, ids);
        let forged = "high risk, owner approval needed: R-web";
        fake.status_by(&h, "risk-check", "failure", forged, "mallory");
    }
    // A fork's merged PR has no ci-ok facts: its R-api block is not a pass.
    fake.pull_from_fork(42, "fix", "other/fork");
    merged(&fake, 42, "2026-11-05T00:00:00Z");
    waiting(&fake, "sha42", "R-api");
    // Re-pushed and open PRs: caught, and not counted.
    pr(&fake, 43, "", &["h43", "h43b"]);
    fake.pull_head_sha(43, "h43b");
    merged(&fake, 43, "2026-11-05T00:00:00Z");
    waiting(&fake, "h43b", "R-web");
    pr(&fake, 44, "", &["h44"]);
    fake.pull_head_sha(44, "h44");
    waiting(&fake, "h44", "R-api");
    let d = metrics(&fake, &["--propose"]);
    assert_eq!(d["curator_loosen"], "off", "{d}");
    assert_eq!(
        (d["days"].as_u64(), d.get("brake")),
        (Some(30), None),
        "{d}"
    );
    let p = d["proposals"].as_array().unwrap();
    assert_eq!(p.len(), 1, "{d}");
    assert_eq!(
        (p[0]["rule"].as_str(), p[0]["blocks"].as_u64()),
        (Some("R-docs"), Some(10))
    );
    assert_eq!(p[0]["prs"], json!((31..=40).collect::<Vec<u64>>()));
    let body = p[0]["body"].as_str().unwrap();
    assert!(
        body.contains("- #31\n") && body.contains("- #40\n"),
        "{body}"
    );
    assert!(p[0]["title"].as_str().unwrap().contains("R-docs"));
    // Plain metrics never carry proposals.
    assert!(metrics(&fake, &[]).get("proposals").is_none());
    // A window that ends before the merges finds nothing.
    let none = metrics(&fake, &["--propose", "--days", "5"]);
    assert_eq!(none["proposals"], json!([]), "{none}");
}

#[test]
fn propose_with_built_in_rules_finds_nothing_every_built_in_is_on_the_floor() {
    let fake = FakeGithub::start();
    for n in 31..=40u64 {
        let h = format!("h{n}");
        pr(&fake, n, "", &[&h]);
        fake.pull_head_sha(n, &h);
        merged(&fake, n, "2026-11-05T00:00:00Z");
        fake.check_run_by(&h, "ci-ok", "success", ACTIONS);
        waiting(&fake, &h, "R-deps, R-ci-review, R-bundle");
    }
    let d = metrics(&fake, &["--propose"]);
    assert_eq!(d["proposals"], json!([]), "{d}");
}

fn task(fake: &FakeGithub, n: u64, epic: u64, claimed: &str, usd: f64) {
    let body = format!(r#"<!-- bosun:task {{"item":"M1","task":"T{n}","epic":{epic}}} -->"#);
    fake.issue(n, &format!("Task {n}"), &[], &body);
    let lease = r#"{"bosun":"lease","session":"s1","origin":"local","expires":"2026-11-20T00:00:00Z","touch":["a"]}"#;
    fake.comment_at(n, "octocat", lease, claimed);
    let spend = format!(r#"{{"bosun":"spend","session":"s1","usd":{usd},"at":"{claimed}"}}"#);
    fake.comment(n, &spend);
}

fn risk(task: u64, rules: &[&str]) -> String {
    let bullets: String = rules.iter().map(|r| format!("- **{r}**: why\n")).collect();
    format!(
        "Closes #{task}\n<!-- bosun:risk -->\n## Risk: high\n\n{bullets}\nOwner: review the head SHA x and approve.\n<!-- /bosun:risk -->"
    )
}

#[test]
fn cost_lead_time_touches_and_blocks() {
    let fake = FakeGithub::start();
    fake.content(
        "governance/owners.toml",
        "main",
        "[[owner]]\ngithub = \"boss\"\n",
    );
    task(&fake, 100, 50, "2026-11-01T00:00:00Z", 2.0);
    task(&fake, 101, 50, "2026-11-02T00:00:00Z", 1.0);
    task(&fake, 102, 51, "2026-11-04T00:00:00Z", 3.0);
    // 20: one head, passed. 21: two heads, caught. 22: no risk note.
    pr(&fake, 20, &risk(100, &["R-gov"]), &["h20"]);
    pr(&fake, 21, &risk(101, &["R-gov"]), &["h21a", "h21b"]);
    pr(&fake, 22, "Closes #102", &["h22"]);
    for h in ["h20", "h21a", "h21b", "h22"] {
        fake.check_run_by(h, "ci-ok", "success", ACTIONS);
    }
    merged(&fake, 20, "2026-11-03T00:00:00Z");
    merged(&fake, 21, "2026-11-05T00:00:00Z");
    merged(&fake, 22, "2026-11-04T12:00:00Z");
    pr(&fake, 23, &risk(0, &["R-gate"]), &["h23"]);
    red(&fake, "h23");
    fake.pull_set(23, "state", json!("closed"));
    fake.pull_set(23, "closed_at", json!("2026-11-06T00:00:00Z"));
    pr(&fake, 24, &risk(0, &["R-gate"]), &[]); // open: skipped
    // A second PR of task 100: its spend counts once.
    pr(&fake, 25, "Closes #100", &[]);
    merged(&fake, 25, "2026-11-04T00:00:00Z");
    for n in [20, 21] {
        fake.pull_as_issue(n, "pr");
    }
    let answer = r#"{"bosun":"answer","session":"s","at":"2026-11-04T00:00:00Z"}"#;
    fake.comment_at(20, "boss", "/bosun approve 20 x", "2026-11-04T00:00:00Z");
    fake.comment_at(100, "boss", answer, "2026-11-04T00:00:00Z");
    fake.comment_at(20, "boss", "thanks", "2026-11-04T01:00:00Z");
    fake.comment_at(21, "octocat", "/bosun approve 21 x", "2026-11-04T01:00:00Z");
    fake.comment_at(21, "boss", "/bosun approve 21 x", "2026-09-01T00:00:00Z");
    let d = metrics(&fake, &[]);
    assert_eq!(d["merged"], 4);
    assert_eq!(d["cost_per_merged_usd"], 1.5);
    assert_eq!(d["epics"], 2);
    assert_eq!(d["lead_time_p50_hours"], 54.0);
    assert_eq!(d["owner_touches"], 2);
    assert_eq!(d["owner_touches_per_day"], 0.07);
    assert_eq!(
        d["blocks"],
        json!([{"rule": "R-gate", "blocks": 1, "caught": 1, "passed": 0},
               {"rule": "R-gov", "blocks": 2, "caught": 1, "passed": 1}])
    );
    assert_eq!(d["days"], 30);
    assert_eq!(d["since"], "2026-10-13T12:00:00Z");
    assert_eq!(
        metrics(&fake, &["--days", "10"])["owner_touches_per_day"],
        0.2
    );
}

#[test]
fn a_pr_that_changes_github_files_never_counts_for_the_rate() {
    let fake = FakeGithub::start();
    pr(&fake, 40, "", &["h40"]);
    red(&fake, "h40");
    // Its own `on: pull_request` workflow could post a green `ci-ok`: not counted.
    for (n, file) in [
        (41, ".github/workflows/ci.yml"),
        (42, ".github/actions/x.yml"),
    ] {
        let h = format!("h{n}");
        pr(&fake, n, "", &[h.as_str()]);
        fake.pull_file(n, file, "modified", 1, 0, None);
        fake.check_run_by(&h, "ci-ok", "success", ACTIONS);
    }
    let s = run_in(&fake, Some("steady"));
    assert_eq!(s["brake"]["window"]["pushed_heads"], 1, "{s}");
    assert_eq!(s["red_ci"]["pushed_heads"], 1);
    assert_eq!(s["brake"]["on"], true);
}

const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

#[test]
fn only_owner_key_signatures_in_the_plain_forms_are_touches() {
    use bosun::core::approve::{Role, signed_message, signed_message_bootstrap};
    use bosun::github::b64;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    let fake = FakeGithub::start();
    let rng = ring::rand::SystemRandom::new();
    let pair = |_| {
        let doc = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap()
    };
    let (own, del) = (pair(0), pair(1));
    let enc = |k: &Ed25519KeyPair| b64::encode(k.public_key().as_ref());
    let keys = format!(
        "[[key]]\nid = \"o1\"\ned25519 = \"{}\"\n\n[[key]]\nid = \"d1\"\ned25519 = \"{}\"\nrole = \"delegate\"\n",
        enc(&own),
        enc(&del)
    );
    fake.content(".bosun/approvers.toml", "refs/heads/main", &keys);
    pr(&fake, 60, "", &[]);
    fake.pull_set(60, "created_at", json!("2026-11-09T00:00:00Z"));
    fake.pull_as_issue(60, "pr");
    let post = |k: &Ed25519KeyPair, id: &str, tail: &str, msg: String| {
        let sig = b64::encode(k.sign(msg.as_bytes()).as_ref());
        let body = format!("/bosun approve {SHA}{tail}\nbosun-sig ed25519 {id} {sig}");
        fake.comment_at(60, "claude", &body, "2026-11-10T00:00:00Z");
    };
    post(
        &own,
        "o1",
        "",
        signed_message(Role::Owner, "acme", "widget", 60, SHA),
    );
    let boot = signed_message_bootstrap("acme", "widget", 60, SHA);
    post(&own, "o1", " bootstrap", boot);
    post(
        &del,
        "d1",
        "",
        signed_message(Role::Delegate, "acme", "widget", 60, SHA),
    );
    post(&own, "o1", "", "a forged message".into());
    assert_eq!(metrics(&fake, &[])["owner_touches"], 1);
    // From the owner login too, a signed comment counts only by its signature.
    fake.content(
        "governance/owners.toml",
        "main",
        "[[owner]]\ngithub = \"boss\"\n",
    );
    let sig = b64::encode(own.sign(b"x").as_ref());
    let body = format!("/bosun approve {SHA} bootstrap\nbosun-sig ed25519 o1 {sig}");
    fake.comment_at(60, "boss", &body, "2026-11-10T00:00:00Z");
    assert_eq!(metrics(&fake, &[])["owner_touches"], 1);
    fake.comment_at(
        60,
        "boss",
        &format!("/bosun approve {SHA}"),
        "2026-11-10T00:00:00Z",
    );
    assert_eq!(metrics(&fake, &[])["owner_touches"], 2);
}
