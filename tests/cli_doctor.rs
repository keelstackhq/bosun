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
}

#[test]
fn a_fully_set_up_repo_is_ok() {
    let (tmp, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let d = tmp.path();
    fake.repo_set("allow_auto_merge", json!(true));
    fake.release("v0.1.0");
    fake.variable("BOSUN_HALT", "0");
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
    write(d, "governance/owners.toml", "owners = [\"adam\"]\n");
    write(d, "docs/setup-script.sh", "#!/bin/sh\n");
    write(d, ".bosun/version", "unreleased\n");
    write(d, ".bosun/bosun.toml", "[slack]\nchannel = \"#bosun\"\n");
    write(d, "plans/bosun.md", "# plan\n");
    let (code, v) = doctor(&fake, d, true);
    assert_eq!(code, 0, "{v}");
    assert!(statuses(&v).iter().all(|s| s == "ok"));
    // Without the Routines confirmation, only step 11 is missing.
    let (code, v) = doctor(&fake, d, false);
    assert_eq!(code, 5);
    assert_eq!(v["error"]["message"], "1 checklist steps missing");
    assert_eq!(statuses(&v)[10], "missing");
}
