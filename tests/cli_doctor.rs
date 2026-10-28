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

#[test]
fn step_9_says_when_auto_merge_is_off() {
    let (tmp, fake) = full_repo("[[owner]]\ngithub = \"adam\"\nslack = \"U1\"\n");
    fake.repo_set("allow_auto_merge", json!(false));
    let (code, v) = doctor(&fake, tmp.path(), true);
    assert_eq!(code, 5, "{v}");
    assert_eq!(statuses(&v)[8], "missing");
    assert!(
        v["data"]["items"][8]["detail"]
            .as_str()
            .is_some_and(|s| s.contains("allow_auto_merge is off")),
        "{v}"
    );
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
    fake.enforce_checks(&["ci-ok", "risk-check", "review-verdict", "main-open"]);
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
    let fake = FakeGithub::start();
    fake.enforce_checks(&[
        "ci-ok",
        "bosun-ok",
        "risk-check",
        "review-verdict",
        "main-open",
    ]);
    step3_on(&fake, dir)
}

fn step3_on(fake: &FakeGithub, dir: &Path) -> Value {
    let (_, v) = doctor(fake, dir, false);
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

fn step3_repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "hoist.toml", FOUR);
    write(
        tmp.path(),
        ".github/workflows/ci.yml",
        "jobs:\n  ci-ok:\n    runs-on: x\n",
    );
    tmp
}

/// hoist.toml only says what hoist would apply; step 3 also reads what the branch enforces.
#[test]
fn step_3_checks_what_the_default_branch_enforces() {
    let tmp = step3_repo();
    let fake = FakeGithub::start();
    // Nothing enforced.
    let item = step3_on(&fake, tmp.path());
    assert_eq!(item["status"], "missing", "{item}");
    assert!(
        item["detail"].as_str().unwrap().contains("risk-check"),
        "{item}"
    );
    assert!(
        item["fix"]
            .as_str()
            .unwrap()
            .contains("hoist apply --area github"),
        "{item}"
    );
    // Partly, through a ruleset.
    fake.enforce_checks(&["ci-ok"]);
    let item = step3_on(&fake, tmp.path());
    assert_eq!(item["status"], "missing", "{item}");
    assert!(
        !item["detail"].as_str().unwrap().contains("ci-ok,"),
        "{item}"
    );
    // The rest through classic protection: the union counts.
    fake.enforce_classic(&["risk-check", "review-verdict", "main-open"]);
    assert_eq!(step3_on(&fake, tmp.path())["status"], "ok");
}

/// A 403 on both endpoints is no evidence of drift: the step keeps its local verdict and says
/// what it could not verify.
#[test]
fn step_3_notes_when_the_enforced_checks_cannot_be_read() {
    let tmp = step3_repo();
    let fake = FakeGithub::start();
    fake.fail_path("/rules/branches/main", 403);
    fake.fail_path("/protection/required_status_checks", 403);
    let item = step3_on(&fake, tmp.path());
    assert_eq!(item["status"], "ok", "{item}");
    let d = item["detail"].as_str().unwrap();
    assert!(d.contains("cannot verify the enforced checks (403"), "{d}");
}

/// One endpoint 403 is no evidence either: a repo protected only by classic protection (often
/// unreadable while /rules/branches answers 200) is not reported missing; the note names the
/// endpoint. Checks the readable endpoint enforces still count.
#[test]
fn step_3_notes_when_only_the_classic_endpoint_is_unreadable() {
    let tmp = step3_repo();
    let fake = FakeGithub::start();
    fake.fail_path("/protection/required_status_checks", 403);
    let item = step3_on(&fake, tmp.path());
    assert_eq!(item["status"], "ok", "{item}");
    let d = item["detail"].as_str().unwrap();
    assert!(d.contains("cannot verify"), "{d}");
    assert!(d.contains("classic branch protection"), "{d}");
    // All enforced through the readable ruleset: ok, no note needed.
    fake.enforce_checks(&["ci-ok", "risk-check", "review-verdict", "main-open"]);
    let item = step3_on(&fake, tmp.path());
    assert_eq!(item["status"], "ok", "{item}");
    assert!(
        !item["detail"].as_str().unwrap().contains("cannot verify"),
        "{item}"
    );
}

/// `--remote` checks the environment of every workflow job: only `main` may deploy to it.
#[cfg(unix)]
mod environments {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const STEPS: &str = "Deployment branches → Selected branches → add `main`";
    const CUSTOM: &str = r#"{"protected_branches": false, "custom_branch_policies": true}"#;

    fn main_only(fake: &FakeGithub, name: &str) {
        let main = json!([{"name": "main", "type": "branch"}]);
        fake.environment(name, serde_json::from_str(CUSTOM).unwrap(), main);
    }

    /// A full repo whose workflows name `release-pr` twice and `splice-bot` as a mapping.
    fn env_repo() -> (tempfile::TempDir, FakeGithub) {
        let (tmp, fake) = full_repo("[[owner]]\ngithub = \"adam\"\nslack = \"U1\"\n");
        let wf = "on:\n  workflow_dispatch:\n    inputs:\n      environment:\n        name: no\njobs:\n  prepare:\n    environment: release-pr\n    steps:\n      - name: x\n  again:\n    environment: \"release-pr\" # twice\n  update:\n    environment:\n      name: splice-bot\n      url: x\n";
        write(tmp.path(), ".github/workflows/rel.yml", wf);
        let hoist = tmp.path().join("fake-hoist");
        let report = r#"{"version":1,"command":"check","ok":true,"exit_code":0,"items":[]}"#;
        std::fs::write(&hoist, format!("#!/bin/sh\necho '{report}'\n")).unwrap();
        std::fs::set_permissions(&hoist, std::fs::Permissions::from_mode(0o755)).unwrap();
        (tmp, fake)
    }

    fn remote(fake: &FakeGithub, dir: &Path) -> (i32, Value) {
        let out = bosun()
            .env("BOSUN_GITHUB_API", fake.url())
            .env("BOSUN_HOIST", dir.join("fake-hoist"))
            .env("BOSUN_ROUTINES_OK", "1")
            .arg("-C")
            .arg(dir)
            .args(["doctor", "--remote"])
            .output()
            .unwrap();
        (out.status.code().unwrap(), one_object(&out.stdout))
    }

    /// `splice-bot` is main-only; returns the exit code, step 5 and the `release-pr` entry.
    fn release_pr(setup: impl FnOnce(&FakeGithub)) -> (i32, Value, Value) {
        let (tmp, fake) = env_repo();
        main_only(&fake, "splice-bot");
        setup(&fake);
        let (code, v) = remote(&fake, tmp.path());
        let envs = v["data"]["environments"].as_array().unwrap().clone();
        assert_eq!(envs.len(), 2, "{v}");
        assert_eq!(envs[1]["status"], "ok", "{v}");
        (code, v["data"]["items"][4].clone(), envs[0].clone())
    }

    /// A warning, never a failure: exit 0, step 5 warns with the entry's detail and fix.
    fn warns(setup: impl FnOnce(&FakeGithub), detail: &str, fix: &str) {
        let (code, step5, env) = release_pr(setup);
        assert_eq!(code, 0, "{step5}");
        assert_eq!(env["name"], "release-pr", "{env}");
        assert_eq!(env["status"], "warn", "{env}");
        let (d, f) = (
            env["detail"].as_str().unwrap(),
            env["fix"].as_str().unwrap(),
        );
        assert!(d.contains(detail), "{d}");
        assert!(f.contains(fix), "{f}");
        assert_eq!(step5["status"], "warn", "{step5}");
        assert!(step5["detail"].as_str().unwrap().contains(d), "{step5}");
        assert!(step5["fix"].as_str().unwrap().contains(f), "{step5}");
    }

    #[test]
    fn an_environment_doctor_cannot_read_warns() {
        let (tmp, fake) = env_repo();
        let dynamic = "jobs:\n  deploy:\n    environment: ${{ inputs.e }}\n  flow:\n    environment: {url: x}\n";
        write(tmp.path(), ".github/workflows/dyn.yml", dynamic);
        main_only(&fake, "release-pr");
        main_only(&fake, "splice-bot");
        let (code, v) = remote(&fake, tmp.path());
        assert_eq!(code, 0, "{v}");
        let envs = v["data"]["environments"].as_array().unwrap();
        for job in ["deploy", "flow"] {
            let e = envs.iter().find(|e| e["job"] == job).expect(job);
            assert_eq!(e["status"], "warn", "{e}");
            let d = e["detail"].as_str().unwrap();
            assert!(
                d.contains(&format!(
                    "cannot read the environment of job {job} in dyn.yml"
                )),
                "{d}"
            );
        }
        assert_eq!(v["data"]["items"][4]["status"], "warn", "{v}");
    }

    #[test]
    fn main_only_environments_are_ok() {
        let (code, step5, env) = release_pr(|f| main_only(f, "release-pr"));
        assert_eq!(code, 0, "{step5}");
        assert_eq!(env["status"], "ok", "{env}");
        assert_eq!(step5["status"], "ok", "{step5}");
        let d = step5["detail"].as_str().unwrap();
        assert!(
            d.contains("only main deploys to release-pr, splice-bot"),
            "{d}"
        );
        // A missing `type` is a branch too.
        let main = json!([{"name": "main"}]);
        let custom = serde_json::from_str(CUSTOM).unwrap();
        let (_, _, env) = release_pr(|f| f.environment("release-pr", custom, main));
        assert_eq!(env["status"], "ok", "{env}");
    }

    #[test]
    fn a_missing_environment_warns() {
        warns(|_| {}, "does not exist", STEPS);
        let (_, _, env) = release_pr(|_| {});
        let fix = env["fix"].as_str().unwrap();
        let new = "Settings → Environments → New environment `release-pr`";
        assert!(fix.contains(new), "{fix}");
        assert!(fix.contains("/acme/widget/settings/environments"), "{fix}");
    }

    #[test]
    fn no_policy_warns_that_any_branch_can_deploy() {
        let fix = format!("Settings → Environments → release-pr → {STEPS}");
        let none = |f: &FakeGithub| f.environment("release-pr", Value::Null, json!([]));
        warns(none, "any branch", &fix);
    }

    #[test]
    fn protected_branches_warn() {
        let p = json!({"protected_branches": true, "custom_branch_policies": false});
        let set = |f: &FakeGithub| f.environment("release-pr", p, json!([]));
        warns(set, "every protected branch", STEPS);
    }

    #[test]
    fn other_branch_policies_warn() {
        let custom: Value = serde_json::from_str(CUSTOM).unwrap();
        let with = |list: Value| {
            let c = custom.clone();
            move |f: &FakeGithub| f.environment("release-pr", c, list)
        };
        let dev = json!([{"name": "main", "type": "branch"}, {"name": "dev", "type": "branch"}]);
        warns(with(dev), "main, dev", STEPS);
        warns(
            with(json!([{"name": "main", "type": "tag"}])),
            "main (tag)",
            STEPS,
        );
        warns(with(json!([])), "no branch", STEPS);
    }

    #[test]
    fn an_unreadable_environment_warns_cannot_verify() {
        let grant = "give the token Environments: read (or Administration: read)";
        let env403 = |f: &FakeGithub| f.fail_path("/environments/release-pr", 403);
        warns(env403, "cannot verify", grant);
        let policies = |f: &FakeGithub| {
            main_only(f, "release-pr");
            f.fail_path("/release-pr/deployment-branch-policies", 403);
        };
        warns(policies, "cannot verify", grant);
        let env500 = |f: &FakeGithub| f.fail_path("/environments/release-pr", 500);
        warns(env500, "cannot verify", "run `bosun doctor --remote` again");
    }
}
