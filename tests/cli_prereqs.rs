mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;
use std::path::Path;

const PLAN: &str = "# Plan: Ship it\n\nproject: x\n\n\
## M1: One\n\narea: core\ndepends: seed\n\nWords.\n\n### Tasks\n\
- T1: a\n  touch: docs/a.md\n- T2: b\n  touch: src/x.rs\n  depends: T1\n\
### Done when\n- UC-M1-1: works\n\n\
## Prerequisites\n\n\
- H1: npm token\n  on: phone\n  steps: add it\n  verify: secret release/NPM_TOKEN\n  blocks: M1/T2\n\
- H2: pc thing\n  on: computer\n  steps: run it\n  blocks: M1\n";
const SECRET: &str = "environments/release/secrets/NPM_TOKEN";
const RULE: &str = "[[rule]]\nid = \"R\"\nwhy = \"w\"\npaths = [\"PATH\"]\napproval = \"APPR\"\n";

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn setup() -> (tempfile::TempDir, FakeGithub) {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    write(tmp.path(), "plans/x.md", PLAN);
    (tmp, fake)
}

/// `bosun prereqs plans/x.md <extra>`.
fn go(fake: &FakeGithub, dir: &Path, extra: &[&str]) -> (i32, Value) {
    let mut a = vec!["-C", dir.to_str().unwrap(), "prereqs", "plans/x.md"];
    a.extend(extra);
    run(fake, "session_a", "2026-10-27T10:00:00Z", &a)
}

fn items(v: &Value) -> &Vec<Value> {
    v["data"]["plans"][0]["items"].as_array().unwrap()
}

fn ids(v: &Value) -> Vec<&str> {
    items(v).iter().map(|i| i["id"].as_str().unwrap()).collect()
}

fn item<'a>(v: &'a Value, id: &str) -> &'a Value {
    items(v).iter().find(|i| i["id"] == id).unwrap()
}

#[test]
fn verifies_through_the_api_and_writes_nothing_without_post() {
    let (tmp, fake) = setup();
    fake.present(SECRET);
    let (code, v) = go(&fake, tmp.path(), &["--no-setup"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        ids(&v),
        ["H2", "H1"],
        "open computer items first, done last"
    );
    assert_eq!(item(&v, "H1")["done"], true);
    assert_eq!(item(&v, "H2")["done"], false);
    assert_eq!(v["data"]["open"], 1);
    assert_eq!(v["data"]["plans"][0]["plan_path"], "plans/x.md");
    assert!(v["data"]["message"].as_str().unwrap().contains('1'));
    assert!(v["data"]["plans"][0]["issue"].is_null());
    assert!(fake.requests().iter().all(|r| r.method == "GET"));
}

#[test]
fn ticks_count_for_owner_items_only() {
    let (tmp, fake) = setup();
    let body = "<!-- bosun:prereqs plans/x.md -->\n\
                - [x] **H2** a <!-- bosun:prereq H2 blocks=M1 verify=owner -->\n\
                - [x] **H1** b <!-- bosun:prereq H1 blocks=M1 verify=api ok -->\n";
    fake.issue(7, "Owner checklist: x", &["bosun:prereqs"], body);
    let (code, v) = go(&fake, tmp.path(), &["--no-setup"]);
    assert_eq!(code, 0, "{v}");
    // The secret is missing: a tick or an old `ok` does not finish it.
    let done = |id| item(&v, id)["done"].as_bool();
    assert_eq!((done("H2"), done("H1")), (Some(true), Some(false)));
    assert_eq!(v["data"]["plans"][0]["issue"]["number"], 7);
}

#[test]
fn a_parse_error_is_exit_1() {
    let (tmp, fake) = setup();
    write(
        tmp.path(),
        "plans/x.md",
        &PLAN.replace("  on: computer\n", ""),
    );
    assert_eq!(go(&fake, tmp.path(), &["--no-setup"]).0, 1);
}

#[test]
fn setup_steps_come_from_doctor_unless_skipped() {
    let (tmp, fake) = setup();
    let (code, v) = go(&fake, tmp.path(), &[]);
    assert_eq!(code, 0, "{v}");
    let d = item(&v, "D2");
    assert_eq!(
        (d["on"].as_str(), d["verify"].as_str()),
        (Some("computer"), Some("doctor"))
    );
    assert!(!d["steps"].as_str().unwrap().is_empty());
    assert!(
        ids(&go(&fake, tmp.path(), &["--no-setup"]).1)
            .iter()
            .all(|i| !i.starts_with('D'))
    );
}

#[test]
fn approvals_and_the_delegate_key() {
    let (tmp, fake) = setup();
    let rule = |path, approval| RULE.replace("PATH", path).replace("APPR", approval);
    let n = |extra: &[&str]| go(&fake, tmp.path(), &[&["--no-setup"], extra].concat());
    write(tmp.path(), ".bosun/risk.toml", &rule("src/**", "owner"));
    write(tmp.path(), "src/cli/risk_check.rs", "");
    let (_, v) = n(&[]);
    let a = &v["data"]["plans"][0]["approvals"][0];
    assert_eq!(
        (a["task"].as_str(), a["needs"].as_str()),
        (Some("M1/T2"), Some("owner"))
    );
    assert!(!ids(&v).contains(&"K1"));
    // A delegate-class task needs a delegate key, until one is enrolled.
    write(tmp.path(), ".bosun/risk.toml", &rule("docs/**", "delegate"));
    assert_eq!(item(&n(&[]).1, "K1")["on"], "computer");
    let key = "[[key]]\nid = \"d\"\ned25519 = \"AAAA\"\nrole = \"delegate\"\n";
    write(tmp.path(), ".bosun/approvers.toml", key);
    assert!(!ids(&n(&[]).1).contains(&"K1"));
    write(tmp.path(), ".bosun/risk.toml", "[[rule]]\nid = 1\n");
    assert_eq!(n(&[]).0, 1);
}

#[test]
fn every_verify_kind_reads_its_api_path() {
    let (tmp, fake) = setup();
    let kinds = "secret S|secret release/S|variable V|variable release/V|environment release|file docs/x.md";
    let mut plan =
        PLAN.split("## Prerequisites").next().unwrap().to_string() + "## Prerequisites\n";
    for (k, v) in kinds.split('|').enumerate() {
        plan += &format!(
            "- H{}: t\n  on: phone\n  steps: s\n  verify: {v}\n  blocks: M1\n",
            k + 1
        );
    }
    write(tmp.path(), "plans/x.md", &plan);
    let done = |v: &Value| {
        (1..=6)
            .map(|k| item(v, &format!("H{k}"))["done"] == true)
            .collect::<Vec<_>>()
    };
    assert_eq!(done(&go(&fake, tmp.path(), &["--no-setup"]).1), [false; 6]);
    for p in [
        "actions/secrets/S",
        "environments/release/secrets/S",
        "environments/release/variables/V",
    ] {
        fake.present(p);
    }
    fake.present("environments/release");
    fake.variable("V", "1");
    fake.content("docs/x.md", "main", "x");
    assert_eq!(done(&go(&fake, tmp.path(), &["--no-setup"]).1), [true; 6]);
    // A 403 (here on both variables and the file) falls back to the tick: nothing ticked yet.
    fake.fail_path("variables/V", 403);
    fake.fail_path("contents/docs/x.md", 403);
    let (_, v) = go(&fake, tmp.path(), &["--no-setup"]);
    assert_eq!(done(&v), [true, true, false, false, true, false]);
    assert_eq!(item(&v, "H6")["api"], false);
}
