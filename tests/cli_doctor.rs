mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};
use std::path::Path;

const T0: &str = "2026-10-22T10:00:00Z";

fn doctor(fake: &FakeGithub, dir: &Path, routines_ok: bool) -> (i32, Value) {
    let mut cmd = bosun();
    cmd.env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", T0)
        .arg("-C")
        .arg(dir)
        .arg("doctor");
    if routines_ok {
        cmd.env("BOSUN_ROUTINES_OK", "1");
    }
    let out = cmd.output().unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn statuses(v: &Value) -> Vec<String> {
    v["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["status"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn an_empty_repo_reports_all_twelve_steps() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let (code, v) = doctor(&fake, tmp.path(), false);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["error"]["kind"], "needs-human");
    let items = v["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 12);
    for (i, it) in items.iter().enumerate() {
        assert_eq!(it["step"], i + 1);
        assert!(!it["fix"].as_str().unwrap().is_empty());
    }
    // Only step 1 (the fake repo is readable) is ok.
    let st = statuses(&v);
    assert_eq!(st[0], "ok");
    assert!(st[1..].iter().all(|s| s == "missing"), "{st:?}");
    assert_eq!(v["error"]["message"], "11 checklist steps missing");
    assert_eq!(v["data"]["routines"].as_array().unwrap().len(), 3);
    let janitor = v["data"]["routines"][0]["prompt"].as_str().unwrap();
    for part in [
        "bosun reap",
        "bosun budget check",
        "`next`",
        "interrupt_session",
    ] {
        assert!(
            janitor.contains(part),
            "janitor prompt lacks {part}: {janitor}"
        );
    }
}

#[test]
fn a_slack_warning_leaves_doctor_at_exit_0() {
    let (tmp, fake) = full_repo("[[owner]]\ngithub = \"adam\"\n");
    let (code, v) = doctor(&fake, tmp.path(), true);
    assert_eq!(code, 0, "{v}");
    assert_eq!(statuses(&v)[6], "warn");
    assert_eq!(v["data"]["missing"], 0);
}

#[test]
fn unparsable_owners_are_named_in_the_warning() {
    let (tmp, fake) = full_repo("owners = [\"adam\"]\n");
    let (_, v) = doctor(&fake, tmp.path(), true);
    let item = &v["data"]["items"][6];
    assert_eq!(item["status"], "warn", "{item}");
    assert_eq!(
        item["detail"],
        "governance/owners.toml does not parse as [[owner]] tables"
    );
}

#[test]
fn a_fully_set_up_repo_is_ok() {
    let (tmp, fake) = full_repo("[[owner]]\ngithub = \"adam\"\nslack = \"U1\"\n");
    let d = tmp.path();
    let (code, v) = doctor(&fake, d, true);
    assert_eq!(code, 0, "{v}");
    assert!(statuses(&v).iter().all(|s| s == "ok"));
    // Without the Routines confirmation, only step 11 is missing.
    let (code, v) = doctor(&fake, d, false);
    assert_eq!(code, 5);
    assert_eq!(v["error"]["message"], "1 checklist steps missing");
    assert_eq!(statuses(&v)[10], "missing");
}

fn full_repo(owners: &str) -> (tempfile::TempDir, FakeGithub) {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let d = tmp.path();
    fake.repo_set("allow_auto_merge", json!(true));
    fake.release("v0.1.0");
    fake.state
        .lock()
        .unwrap()
        .labels
        .push(("bosun:halt".into(), "B60205".into()));
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    fake.issue(2, "task", &[], "");
    fake.comment_by(2, "claude[bot]", "On it.");
    write(
        d,
        "hoist.toml",
        "[github]\nrequired_checks = [\"ci-ok\", \"risk-check\", \"review-verdict\", \"main-open\"]\n",
    );
    write(
        d,
        ".github/workflows/ci.yml",
        "jobs:\n  ci-ok:\n    runs-on: x\n",
    );
    write(d, "governance/owners.toml", owners);
    write(d, "docs/setup-script.sh", "#!/bin/sh\n");
    write(d, ".bosun/version", "unreleased\n");
    write(d, ".bosun/bosun.toml", "[slack]\nchannel = \"#bosun\"\n");
    write(d, "plans/bosun.md", "# plan\n");
    (tmp, fake)
}

const FOUR: &str =
    "[github]\nrequired_checks = [\"ci-ok\", \"risk-check\", \"review-verdict\", \"main-open\"]\n";
const FIVE: &str = "[github]\nrequired_checks = [\"ci-ok\", \"bosun-ok\", \"risk-check\", \"review-verdict\", \"main-open\"]\n";

fn step3(dir: &Path) -> Value {
    let (_, v) = doctor(&FakeGithub::start(), dir, false);
    v["data"]["items"][2].clone()
}

#[test]
fn step_3_finds_ci_ok_in_any_workflow() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "hoist.toml", FOUR);
    write(
        tmp.path(),
        ".github/workflows/bosun.yml",
        "jobs:\n  ci-ok:\n    runs-on: x\n",
    );
    assert_eq!(step3(tmp.path())["status"], "ok");
}

#[test]
fn step_3_wants_bosun_ok_in_hoist_toml_when_a_workflow_has_it() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    write(d, "hoist.toml", FOUR);
    write(
        d,
        ".github/workflows/ci.yml",
        "jobs:\n  ci-ok:\n    runs-on: x\n",
    );
    write(
        d,
        ".github/workflows/bosun.yml",
        "jobs:\n  bosun-ok:\n    runs-on: x\n",
    );
    let item = step3(d);
    assert_eq!(item["status"], "missing", "{item}");
    assert!(
        item["detail"].as_str().unwrap().contains("bosun-ok"),
        "{item}"
    );
    assert!(item["fix"].as_str().unwrap().contains("bosun-ok"), "{item}");
    write(d, "hoist.toml", FIVE);
    assert_eq!(step3(d)["status"], "ok");
}

#[test]
fn step_3_reads_jobs_not_other_keys_and_accepts_comments_and_yaml() {
    let tmp = tempfile::tempdir().unwrap();
    let d = tmp.path();
    write(d, "hoist.toml", FOUR);
    // `ci-ok:` under `on:` is not a job.
    write(
        d,
        ".github/workflows/ci.yml",
        "on:\n  ci-ok:\n    x: 1\njobs:\n  build:\n    runs-on: x\n",
    );
    assert_eq!(step3(d)["status"], "missing");
    write(
        d,
        ".github/workflows/bosun.yaml",
        "jobs: # gates\n  ci-ok: # aggregate\n    runs-on: x\n",
    );
    assert_eq!(step3(d)["status"], "ok");
}

fn slack_step(owners: &str) -> Value {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    write(tmp.path(), "governance/owners.toml", owners);
    write(
        tmp.path(),
        ".bosun/bosun.toml",
        "[slack]\nchannel = \"#bosun\"\n",
    );
    let (_, v) = doctor(&fake, tmp.path(), false);
    v["data"]["items"][6].clone()
}

#[test]
fn doctor_warns_when_no_owner_has_a_slack_id() {
    let item = slack_step("[[owner]]\ngithub = \"adam\"\n");
    assert_eq!(item["status"], "warn", "{item}");
    assert_eq!(
        item["fix"],
        "add `slack = \"U…\"` to your [[owner]] table in governance/owners.toml"
    );
}

#[test]
fn doctor_does_not_warn_when_an_owner_has_a_slack_id() {
    let item = slack_step("[[owner]]\ngithub = \"adam\"\nslack = \"U1\"\n");
    assert_eq!(item["status"], "ok", "{item}");
}

fn step2(fake: &FakeGithub) -> Value {
    let tmp = tempfile::tempdir().unwrap();
    let (_, v) = doctor(fake, tmp.path(), false);
    v["data"]["items"][1].clone()
}

/// The app acts as the owner's login: only `performed_via_github_app` shows it.
#[test]
fn step_2_sees_the_app_through_performed_via_github_app() {
    let fake = FakeGithub::start();
    fake.issue(2, "task", &[], "");
    assert_eq!(step2(&fake)["status"], "missing");
    fake.issue_via_app(2, "claude");
    assert_eq!(step2(&fake)["status"], "ok");

    let fake = FakeGithub::start();
    fake.issue(2, "task", &[], "");
    fake.comment_by(2, "adam", "On it.");
    fake.last_comment(|c| c.via_app = Some("Claude".into()));
    assert_eq!(step2(&fake)["status"], "ok");

    let fake = FakeGithub::start();
    fake.issue(2, "task", &[], "");
    fake.issue_via_app(2, "dependabot");
    assert_eq!(step2(&fake)["status"], "missing");
}
