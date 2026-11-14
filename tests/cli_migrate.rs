//! `bosun migrate`: one PR that moves `[mode]` one rung (or straight to the target), with the
//! survey, the version and registry steps, and `--finish` after the merge.
mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::npm::{Registry, repo as npm_repo};
use common::{bosun, one_object};
use serde_json::{Value, json};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct Env {
    work: tempfile::TempDir,
    origin: tempfile::TempDir,
}

/// A repo on `main` with a bare local origin; `files` are committed first.
fn env(files: &[(&str, &str)]) -> Env {
    let work = tempfile::tempdir().unwrap();
    let origin = tempfile::tempdir().unwrap();
    git(origin.path(), &["init", "-q", "--bare", "-b", "main"]);
    git(work.path(), &["init", "-q", "-b", "main"]);
    for (rel, text) in files {
        let p = work.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    git(work.path(), &["add", "-A"]);
    git(work.path(), &["commit", "-q", "-m", "x", "--allow-empty"]);
    let url = origin.path().to_str().unwrap();
    git(work.path(), &["remote", "add", "origin", url]);
    git(work.path(), &["push", "-q", "origin", "main"]);
    Env { work, origin }
}

const CONFIG: &str = "project = \"widget\"\n";
const VERSION: (&str, &str) = (".bosun/version", "0.14.0\n");
const BOSUN_TOML: &str = ".bosun/bosun.toml";
const READY: &str = "bosun:state/ready";

fn plain() -> Env {
    env(&[(BOSUN_TOML, CONFIG), VERSION])
}

fn migrate(env: &Env, fake: &FakeGithub, args: &[&str]) -> (i32, Value) {
    migrate_with(env, fake, args, &[("BOSUN_MIGRATE_METRICS_TIMEOUT", "0")])
}

fn migrate_with(
    env: &Env,
    fake: &FakeGithub,
    args: &[&str],
    vars: &[(&str, &str)],
) -> (i32, Value) {
    let mut cmd = bosun();
    cmd.env("BOSUN_GITHUB_API", fake.url());
    for (k, v) in vars {
        cmd.env(k, v);
    }
    let out = cmd
        .arg("-C")
        .arg(env.work.path())
        .arg("migrate")
        .args(args)
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), v)
}

fn writes(fake: &FakeGithub) -> usize {
    fake.requests().iter().filter(|r| r.method != "GET").count()
}

fn read(env: &Env, rel: &str) -> String {
    std::fs::read_to_string(env.work.path().join(rel)).unwrap_or_default()
}

#[test]
fn a_dry_run_names_one_rung_and_writes_nothing() {
    let fake = FakeGithub::start();
    let e = plain();
    let (code, v) = migrate(&e, &fake, &["--to", "booster", "--dry-run"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["from"], "gatekeeper");
    assert_eq!(d["to"], "booster");
    assert_eq!(d["step"], "steady");
    assert_eq!(d["direct"], false);
    assert_eq!(d["lighter"], true);
    assert_eq!(d["dry_run"], true);
    assert_eq!(d["files"], json!([BOSUN_TOML]));
    assert_eq!(d["pr"], Value::Null);
    assert_eq!(read(&e, BOSUN_TOML), CONFIG);
    assert_eq!(git(e.work.path(), &["branch", "--show-current"]), "main");
    assert_eq!(writes(&fake), 0);
    let then = d["then"].to_string();
    assert!(
        then.contains("rerun `bosun migrate --to booster`"),
        "{then}"
    );
    assert!(d["owner"]["steps"].to_string().contains("Janitor"));
}

#[test]
fn direct_goes_to_the_target_and_the_body_records_the_choice() {
    let fake = FakeGithub::start();
    let e = plain();
    let (code, v) = migrate(&e, &fake, &["--to", "booster", "--direct"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["step"], "booster");
    assert_eq!(v["data"]["direct"], true);
    let pr = v["data"]["pr"]["number"].as_u64().unwrap();
    let body = fake.pull_body(pr);
    assert!(
        body.contains("The owner chose --direct (skips steady)"),
        "{body}"
    );
    assert!(!body.contains("orchestrator command"), "{body}");
    assert!(body.contains("R-weaken"), "{body}");
    assert!(body.contains("<!-- bosun:migrate booster -->"), "{body}");
    assert!(read(&e, BOSUN_TOML).contains("[mode]\nname = \"booster\"\n"));
}

#[test]
fn a_full_run_opens_a_draft_pr_on_a_pushed_branch() {
    let fake = FakeGithub::start();
    let e = plain();
    let (code, v) = migrate(&e, &fake, &["--to", "steady"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["branch"], "bosun/migrate-steady");
    assert_eq!(d["reused"], false);
    assert_eq!(d["changed"], true);
    let pr = d["pr"]["number"].as_u64().unwrap();
    assert!(fake.pull_is_draft(pr));
    assert_eq!(fake.pull_head(pr), d["pr"]["head_sha"].as_str().unwrap());
    assert_eq!(
        read(&e, BOSUN_TOML),
        "project = \"widget\"\n\n[mode]\nname = \"steady\"\n"
    );
    let pushed = git(
        e.origin.path(),
        &["branch", "--list", "bosun/migrate-steady"],
    );
    assert!(pushed.contains("bosun/migrate-steady"), "{pushed}");
    let log = git(e.work.path(), &["log", "-1", "--format=%s"]);
    assert_eq!(log, "chore: bosun mode gatekeeper -> steady");
    assert!(d["then"].to_string().contains("bosun pr --pr"));
    assert!(
        d["owner"]["approval"]
            .as_str()
            .unwrap()
            .contains("head SHA")
    );
    // Never a merge.
    assert!(fake.requests().iter().all(|r| !r.path.ends_with("/merge")));
}

#[test]
fn the_registry_pin_is_written_from_the_live_issue() {
    let fake = FakeGithub::start();
    fake.issue(3, "bosun registry", &["bosun:registry"], "");
    let e = plain();
    let (code, v) = migrate(&e, &fake, &["--to", "steady"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(read(&e, ".bosun/registry"), "3\n");
    assert_eq!(
        v["data"]["survey"]["registry"],
        json!({"pinned": null, "live": 3})
    );
    let files = v["data"]["files"].to_string();
    assert!(
        files.contains(".bosun/registry") && files.contains(BOSUN_TOML),
        "{files}"
    );
}

#[test]
fn two_open_registries_and_no_pin_is_a_conflict() {
    let fake = FakeGithub::start();
    fake.issue(3, "bosun registry", &["bosun:registry"], "");
    fake.issue(8, "bosun registry again", &["bosun:registry"], "");
    let e = plain();
    let (code, v) = migrate(&e, &fake, &["--to", "steady"]);
    assert_eq!(code, 3, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("registry"));
    assert_eq!(read(&e, BOSUN_TOML), CONFIG);
    assert_eq!(writes(&fake), 0);
}

#[test]
fn a_pin_file_settles_two_registries() {
    let fake = FakeGithub::start();
    fake.issue(3, "bosun registry", &["bosun:registry"], "");
    fake.issue(8, "bosun registry again", &["bosun:registry"], "");
    let e = env(&[(BOSUN_TOML, CONFIG), VERSION, (".bosun/registry", "8\n")]);
    let (code, v) = migrate(&e, &fake, &["--to", "steady", "--dry-run"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["survey"]["registry"],
        json!({"pinned": 8, "live": 3})
    );
    assert_eq!(v["data"]["files"], json!([BOSUN_TOML]));
}

#[test]
fn an_old_version_in_a_bosun_channel_repo_needs_splice() {
    let fake = FakeGithub::start();
    let e = env(&[
        (BOSUN_TOML, CONFIG),
        (".bosun/version", "0.13.0\n"),
        (".splice/channels/bosun.toml", "x = 1\n"),
    ]);
    let (code, v) = migrate(&e, &fake, &["--to", "steady"]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("splice update --channel bosun")
    );
    assert_eq!(read(&e, BOSUN_TOML), CONFIG);
}

#[test]
fn an_old_version_is_pinned_and_its_files_join_the_commit() {
    let fake = FakeGithub::start();
    let reg = Registry::start(
        &[
            ("splice", "0.4.0"),
            ("hoist", "0.4.1"),
            ("berth", "0.1.0"),
            ("bosun", "0.15.0"),
        ],
        None,
        None,
    );
    let r = npm_repo();
    let mut files: Vec<(String, String)> = Vec::new();
    for rel in [
        "docs/setup-script.sh",
        "channel/pieces/core/files/docs/setup-script.sh.jinja",
    ] {
        files.push((
            rel.into(),
            std::fs::read_to_string(r.path().join(rel)).unwrap(),
        ));
    }
    let mut all: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    all.push((BOSUN_TOML, CONFIG));
    all.push((".bosun/version", "0.9.0\n"));
    let e = env(&all);
    let (code, v) = migrate_with(
        &e,
        &fake,
        &["--to", "steady"],
        &[
            ("BOSUN_MIGRATE_METRICS_TIMEOUT", "0"),
            ("BOSUN_NPM_REGISTRY", &reg.base),
        ],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(read(&e, ".bosun/version"), "0.15.0\n");
    let files = v["data"]["files"].to_string();
    assert!(
        files.contains(".bosun/version") && files.contains("docs/setup-script.sh"),
        "{files}"
    );
    assert_eq!(git(e.work.path(), &["status", "--porcelain"]), "");
}

#[test]
fn a_failed_pin_names_the_pin_step() {
    let fake = FakeGithub::start();
    let e = pin_env(Some("0.9.0\n"));
    let (code, v) = migrate_with(
        &e,
        &fake,
        &["--to", "steady"],
        &[
            ("BOSUN_MIGRATE_METRICS_TIMEOUT", "0"),
            ("BOSUN_NPM_REGISTRY", "http://127.0.0.1:9"),
        ],
    );
    // The child's own exit code (a network error) comes through.
    assert_eq!(code, 1, "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(msg.contains("bosun pin"), "{msg}");
    // Not below the minimum: the child's own message and fix come through.
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(!fix.contains("publishes a bosun release"), "{fix}");
    assert!(msg.len() > "the pin step (`bosun pin --latest --tool bosun`) failed: ".len());
    assert_eq!(read(&e, BOSUN_TOML), CONFIG);
    assert_eq!(git(e.work.path(), &["status", "--porcelain"]), "");
}

fn setup_script_files() -> Vec<(String, String)> {
    let r = npm_repo();
    [
        "docs/setup-script.sh",
        "channel/pieces/core/files/docs/setup-script.sh.jinja",
    ]
    .iter()
    .map(|rel| {
        (
            rel.to_string(),
            std::fs::read_to_string(r.path().join(rel)).unwrap(),
        )
    })
    .collect()
}

fn pin_env(version: Option<&str>) -> Env {
    let files = setup_script_files();
    let mut all: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    all.push((BOSUN_TOML, CONFIG));
    if let Some(v) = version {
        all.push((".bosun/version", v));
    }
    env(&all)
}

fn registry(bosun_version: &str) -> Registry {
    Registry::start(
        &[
            ("splice", "0.4.0"),
            ("hoist", "0.4.1"),
            ("berth", "0.1.0"),
            ("bosun", bosun_version),
        ],
        None,
        None,
    )
}

fn with_registry(reg: &Registry) -> [(&'static str, String); 2] {
    [
        ("BOSUN_MIGRATE_METRICS_TIMEOUT", "0".to_string()),
        ("BOSUN_NPM_REGISTRY", reg.base.clone()),
    ]
}

fn run_pinned(e: &Env, fake: &FakeGithub, reg: &Registry, args: &[&str]) -> (i32, Value) {
    let vars = with_registry(reg);
    let vars: Vec<(&str, &str)> = vars.iter().map(|(a, b)| (*a, b.as_str())).collect();
    migrate_with(e, fake, args, &vars)
}

#[test]
fn a_newest_release_below_the_minimum_is_the_release_fix_and_leaves_the_tree_clean() {
    let fake = FakeGithub::start();
    let reg = registry("0.13.0");
    let e = pin_env(Some("0.9.0\n"));
    let (code, v) = run_pinned(&e, &fake, &reg, &["--to", "steady"]);
    assert_eq!(code, 3, "{v}");
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("publishes a bosun release"), "{fix}");
    // The pin wrote files before the check failed: all of it is undone.
    assert_eq!(git(e.work.path(), &["status", "--porcelain"]), "");
    assert_eq!(read(&e, ".bosun/version"), "0.9.0\n");
    assert_eq!(git(e.work.path(), &["branch", "--show-current"]), "main");
    assert_eq!(writes(&fake), 0);
}

#[test]
fn a_repo_without_a_version_file_gets_the_pinned_version_written() {
    let fake = FakeGithub::start();
    let reg = registry("0.15.0");
    let e = pin_env(None);
    let (code, v) = run_pinned(&e, &fake, &reg, &["--to", "steady"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(read(&e, ".bosun/version"), "0.15.0\n");
    assert!(v["data"]["files"].to_string().contains(".bosun/version"));
    assert_eq!(git(e.work.path(), &["status", "--porcelain"]), "");
    let shown = git(
        e.work.path(),
        &["show", "--stat", "--format=", "bosun/migrate-steady"],
    );
    assert!(shown.contains(".bosun/version"), "{shown}");
}

#[test]
fn a_repo_without_a_version_file_refuses_a_newest_release_below_the_minimum() {
    let fake = FakeGithub::start();
    let reg = registry("0.13.0");
    let e = pin_env(None);
    let (code, v) = run_pinned(&e, &fake, &reg, &["--to", "steady"]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("publishes a bosun release")
    );
    assert_eq!(git(e.work.path(), &["status", "--porcelain"]), "");
    assert!(!e.work.path().join(".bosun/version").exists());
}

#[test]
fn a_failed_commit_restores_the_tree_and_the_branch() {
    let fake = FakeGithub::start();
    let e = plain();
    let hook = e.work.path().join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\necho nope >&2\nexit 1\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (code, v) = migrate(&e, &fake, &["--to", "steady"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(git(e.work.path(), &["status", "--porcelain"]), "");
    assert_eq!(read(&e, BOSUN_TOML), CONFIG);
    assert_eq!(git(e.work.path(), &["branch", "--show-current"]), "main");
    assert_eq!(
        git(e.work.path(), &["branch", "--list", "bosun/migrate-*"]),
        ""
    );
    assert_eq!(writes(&fake), 0);
}

#[test]
fn a_local_head_that_is_not_origins_is_a_conflict_before_any_write() {
    let fake = FakeGithub::start();
    let e = plain();
    git(
        e.work.path(),
        &["commit", "-q", "--allow-empty", "-m", "local only"],
    );
    let (code, v) = migrate(&e, &fake, &["--to", "steady"]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("git pull --ff-only"),
        "{v}"
    );
    assert_eq!(read(&e, BOSUN_TOML), CONFIG);
    assert_eq!(git(e.work.path(), &["status", "--porcelain"]), "");
    assert_eq!(writes(&fake), 0);
}

#[test]
fn an_unreachable_origin_is_an_error_with_no_credentials_in_it() {
    let fake = FakeGithub::start();
    let e = plain();
    git(
        e.work.path(),
        &[
            "remote",
            "set-url",
            "origin",
            "http://bob:hunter2@127.0.0.1:9/x.git",
        ],
    );
    let (code, v) = migrate(&e, &fake, &["--to", "steady"]);
    assert_eq!(code, 1, "{v}");
    assert!(!v.to_string().contains("hunter2"), "{v}");
    assert_eq!(git(e.work.path(), &["status", "--porcelain"]), "");
}

#[test]
fn a_hanging_fetch_is_killed_at_the_limit() {
    let fake = FakeGithub::start();
    let e = plain();
    let hang = e.origin.path().join("hang-ssh.sh");
    std::fs::write(&hang, "#!/bin/sh\nsleep 30\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hang, std::fs::Permissions::from_mode(0o755)).unwrap();
    git(
        e.work.path(),
        &["remote", "set-url", "origin", "ssh://example.invalid/x.git"],
    );
    let t = std::time::Instant::now();
    let (code, v) = migrate_with(
        &e,
        &fake,
        &["--to", "steady"],
        &[
            ("BOSUN_MIGRATE_METRICS_TIMEOUT", "0"),
            ("BOSUN_MIGRATE_GIT_TIMEOUT", "1"),
            ("GIT_SSH_COMMAND", hang.to_str().unwrap()),
        ],
    );
    assert!(t.elapsed().as_secs() < 20, "took {:?}", t.elapsed());
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("timed out"),
        "{v}"
    );
    assert_eq!(git(e.work.path(), &["status", "--porcelain"]), "");
}

#[test]
fn an_open_migrate_pr_for_another_step_is_a_conflict_naming_it() {
    let fake = FakeGithub::start();
    fake.pull(
        12,
        "chore: bosun mode gatekeeper -> steady",
        "bosun/migrate-steady",
        "",
    );
    let e = plain();
    let (code, v) = migrate(&e, &fake, &["--to", "booster", "--direct"]);
    assert_eq!(code, 3, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("#12"));
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("merge or close #12")
    );
    assert_eq!(writes(&fake), 0);
    assert_eq!(read(&e, BOSUN_TOML), CONFIG);
}

#[test]
fn only_a_bosun_migrate_dash_branch_counts_as_the_migration_pr() {
    let fake = FakeGithub::start();
    fake.pull(12, "x", "bosun/migrate", "");
    fake.pull(13, "x", "bosun/migrated-things", "");
    let e = plain();
    let (code, v) = migrate(&e, &fake, &["--to", "steady"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["reused"], false);
}

#[test]
fn a_direct_move_onto_the_next_rung_has_no_skip_note() {
    let fake = FakeGithub::start();
    let e = plain();
    let (code, v) = migrate(&e, &fake, &["--to", "steady", "--direct"]);
    assert_eq!(code, 0, "{v}");
    let pr = v["data"]["pr"]["number"].as_u64().unwrap();
    assert!(!fake.pull_body(pr).contains("--direct"));
}

#[test]
fn an_open_migrate_pr_is_reused_and_nothing_is_written() {
    let fake = FakeGithub::start();
    fake.pull(
        12,
        "chore: bosun mode gatekeeper -> steady",
        "bosun/migrate-steady",
        "",
    );
    let e = plain();
    let (code, v) = migrate(&e, &fake, &["--to", "steady"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["reused"], true);
    assert_eq!(v["data"]["pr"]["number"], 12);
    assert_eq!(v["data"]["pr"]["head_sha"], "sha12");
    assert_eq!(read(&e, BOSUN_TOML), CONFIG);
    assert_eq!(writes(&fake), 0);
    assert_eq!(git(e.work.path(), &["branch", "--show-current"]), "main");
}

#[test]
fn a_fork_pr_named_like_the_branch_is_not_ours() {
    let fake = FakeGithub::start();
    fake.pull_from_fork(12, "bosun/migrate-steady", "mallory/widget");
    let e = plain();
    let (code, v) = migrate(&e, &fake, &["--to", "steady"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["reused"], false);
}

#[test]
fn a_dirty_tree_or_another_branch_stops_before_any_write() {
    let fake = FakeGithub::start();
    let e = plain();
    std::fs::write(e.work.path().join("stray.txt"), "x").unwrap();
    let (code, v) = migrate(&e, &fake, &["--to", "steady"]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("commit or stash")
    );
    assert_eq!(read(&e, BOSUN_TOML), CONFIG);
    std::fs::remove_file(e.work.path().join("stray.txt")).unwrap();
    git(e.work.path(), &["switch", "-q", "-c", "feature"]);
    let (code, v) = migrate(&e, &fake, &["--to", "steady"]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("git switch main")
    );
    assert_eq!(writes(&fake), 0);
}

#[test]
fn a_taken_branch_gets_a_dated_name() {
    let fake = FakeGithub::start();
    let e = plain();
    git(e.work.path(), &["branch", "bosun/migrate-steady"]);
    let (code, v) = migrate(&e, &fake, &["--to", "steady"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["branch"], "bosun/migrate-steady-20261022");
}

#[test]
fn a_metrics_timeout_leaves_the_baseline_empty_and_still_succeeds() {
    let fake = FakeGithub::start();
    let e = plain();
    let (code, v) = migrate(&e, &fake, &["--to", "steady", "--dry-run"]);
    assert_eq!(code, 0, "{v}");
    let s = &v["data"]["survey"];
    assert_eq!(s["baseline"], Value::Null);
    assert!(s["notes"].to_string().contains("baseline"), "{s}");
    assert_eq!(s["binary"], env!("CARGO_PKG_VERSION"));
    assert_eq!(s["version"], "0.14.0");
}

#[test]
fn the_baseline_is_the_metrics_data_when_it_arrives() {
    let fake = FakeGithub::start();
    let e = plain();
    let (code, v) = migrate_with(
        &e,
        &fake,
        &["--to", "steady", "--dry-run"],
        &[("BOSUN_MIGRATE_METRICS_TIMEOUT", "60")],
    );
    assert_eq!(code, 0, "{v}");
    assert!(
        v["data"]["survey"]["baseline"].is_object(),
        "{}",
        v["data"]["survey"]
    );
}

#[test]
fn the_survey_lists_unmarked_epics_and_the_plans_of_marked_ones() {
    let fake = FakeGithub::start();
    fake.issue(10, "old epic", &["bosun:source/plan"], "no marker here");
    fake.issue(
        11,
        "new epic",
        &["bosun:source/plan"],
        "<!-- bosun:epic {\"item\":\"M1\",\"plan\":\"plans/widget.md\"} -->",
    );
    for (n, epic) in [(20, 10), (21, 11)] {
        fake.issue(
            n,
            "task",
            &["bosun:source/plan", READY],
            &format!("<!-- bosun:task {{\"item\":\"M1\",\"task\":\"T1\",\"epic\":{epic}}} -->"),
        );
    }
    let e = plain();
    let (code, v) = migrate(&e, &fake, &["--to", "steady", "--dry-run"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["survey"]["epics_unmarked"], json!([10]));
    assert_eq!(v["data"]["survey"]["plans"], json!(["plans/widget.md"]));
}

#[test]
fn the_hull_lock_and_the_bosun_channel_are_surveyed() {
    let fake = FakeGithub::start();
    let e = env(&[
        (BOSUN_TOML, CONFIG),
        VERSION,
        (".splice/lock.toml", "[hull]\nversion = \"1.2.3\"\n"),
    ]);
    let (code, v) = migrate(&e, &fake, &["--to", "steady", "--dry-run"]);
    assert_eq!(code, 0, "{v}");
    let s = &v["data"]["survey"];
    assert_eq!(s["hull"], json!({"lock": true, "version": "1.2.3"}));
    assert_eq!(s["bosun_channel"], false);
}

#[test]
fn a_target_that_is_already_set_changes_nothing() {
    let fake = FakeGithub::start();
    let e = env(&[
        (
            BOSUN_TOML,
            "project = \"widget\"\n\n[mode]\nname = \"steady\"\n",
        ),
        VERSION,
        (".bosun/registry", "3\n"),
    ]);
    let (code, v) = migrate(&e, &fake, &["--to", "steady"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["changed"], false);
    assert_eq!(v["data"]["pr"], Value::Null);
    assert!(
        v["data"]["then"]
            .to_string()
            .contains("bosun migrate --finish")
    );
    assert_eq!(writes(&fake), 0);
}

#[test]
fn a_tighter_target_is_one_step_and_not_r_weaken() {
    let fake = FakeGithub::start();
    let e = env(&[
        (
            BOSUN_TOML,
            "project = \"widget\"\n\n[mode]\nname = \"autonomous\"\n",
        ),
        VERSION,
    ]);
    let (code, v) = migrate(&e, &fake, &["--to", "gatekeeper"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["step"], "gatekeeper");
    assert_eq!(v["data"]["lighter"], false);
    let pr = v["data"]["pr"]["number"].as_u64().unwrap();
    assert!(!fake.pull_body(pr).contains("R-weaken"));
}

#[test]
fn migrate_needs_a_target_or_finish() {
    let fake = FakeGithub::start();
    let e = plain();
    let (code, v) = migrate(&e, &fake, &[]);
    assert_eq!(code, 2, "{v}");
    let (code, _) = migrate(&e, &fake, &["--finish", "--to", "steady"]);
    assert_eq!(code, 2);
    let (code, _) = migrate(&e, &fake, &["--to", "steady", "--plan", "p.md"]);
    assert_eq!(code, 2);
    let (code, _) = migrate(&e, &fake, &["--to", "fast"]);
    assert_eq!(code, 2);
}

const PLAN: &str = "# Plan: widget

project: widget

## M1: Lifecycle

area: lifecycle

### Tasks

- T1: State machine
  touch: src/core/state.rs

### Done when

- UC-M1-1: a task moves planned -> merged.
";

fn finish_env() -> (Env, FakeGithub) {
    let fake = FakeGithub::start();
    let toml = "project = \"widget\"\n\n[mode]\nname = \"steady\"\n";
    fake.content(BOSUN_TOML, "main", toml);
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    let e = env(&[(BOSUN_TOML, toml), VERSION, ("plans/widget.md", PLAN)]);
    (e, fake)
}

#[test]
fn finish_joins_and_amends_each_plan_and_a_second_run_changes_nothing() {
    let (e, fake) = finish_env();
    fake.issue(
        11,
        "epic M1",
        &["bosun:source/plan"],
        "<!-- bosun:epic {\"item\":\"M1\",\"plan\":\"plans/widget.md\"} -->",
    );
    fake.issue(
        21,
        "task",
        &["bosun:source/plan", READY],
        "<!-- bosun:task {\"item\":\"M1\",\"task\":\"T1\",\"epic\":11} -->",
    );
    let (code, v) = migrate(&e, &fake, &["--finish"]);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    assert_eq!(d["joined"], true);
    assert_eq!(d["mode"], "steady");
    assert_eq!(d["amended"].as_array().unwrap().len(), 1, "{d}");
    assert_eq!(d["amended"][0]["plan"], "plans/widget.md");
    assert_eq!(d["amended"][0]["ok"], true);
    assert!(
        fake.comments_of(1)
            .iter()
            .any(|c| c.body.contains("\"bosun\":\"join\"")),
        "joined in the registry"
    );
    let comments = fake.comments_of(1).len();
    let (code, v) = migrate(&e, &fake, &["--finish"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(fake.comments_of(1).len(), comments, "idempotent join");
    assert_eq!(v["data"]["amended"][0]["ok"], true);
}

#[test]
fn finish_takes_extra_plans_and_notes_unmarked_epics_without_one() {
    let (e, fake) = finish_env();
    fake.issue(10, "old epic", &["bosun:source/plan"], "no marker");
    fake.issue(
        20,
        "task",
        &["bosun:source/plan", READY],
        "<!-- bosun:task {\"item\":\"M1\",\"task\":\"T1\",\"epic\":10} -->",
    );
    let (code, v) = migrate(&e, &fake, &["--finish"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["epics_unmarked"], json!([10]));
    let notes = v["data"]["notes"].to_string();
    assert!(
        notes.contains("bosun migrate --finish --plan <file>"),
        "{notes}"
    );
    let (code, v) = migrate(&e, &fake, &["--finish", "--plan", "plans/widget.md"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["amended"][0]["plan"], "plans/widget.md");
}

#[test]
fn finish_reports_a_failed_amend_with_its_fix() {
    let (e, fake) = finish_env();
    let (code, v) = migrate(&e, &fake, &["--finish", "--plan", "plans/missing.md"]);
    assert_eq!(code, 1, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert_eq!(v["data"]["amended"][0]["ok"], false);
    assert_eq!(v["data"]["amended"][0]["exit_code"], 1);
}

#[test]
fn finish_skips_the_amend_while_the_budget_feature_is_off() {
    let fake = FakeGithub::start();
    fake.content(BOSUN_TOML, "main", CONFIG);
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    let e = plain();
    let (code, v) = migrate(&e, &fake, &["--finish", "--plan", "plans/widget.md"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["amended"], json!([]));
    assert_eq!(v["data"]["joined"], true);
}

#[test]
fn finish_waits_while_a_migration_pr_is_open() {
    let (e, fake) = finish_env();
    fake.pull(
        12,
        "chore: bosun mode gatekeeper -> steady",
        "bosun/migrate-steady",
        "",
    );
    let (code, v) = migrate(&e, &fake, &["--finish"]);
    assert_eq!(code, 3, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("PR #12 not merged yet")
    );
    assert_eq!(
        v["error"]["fix"],
        "wait for the merge, then rerun `bosun migrate --finish`"
    );
    assert_eq!(writes(&fake), 0);
}

#[test]
fn finish_reports_the_mode_of_the_default_branch() {
    let fake = FakeGithub::start();
    fake.content(
        BOSUN_TOML,
        "main",
        "project = \"widget\"\n\n[mode]\nname = \"booster\"\n",
    );
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
    // The checkout still has the old config: the merged one decides.
    let e = plain();
    let (code, v) = migrate(&e, &fake, &["--finish"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["mode"], "booster");
}

#[test]
fn a_user_file_survives_when_untracked_files_are_hidden() {
    let fake = FakeGithub::start();
    let e = plain();
    git(
        e.work.path(),
        &["config", "status.showUntrackedFiles", "no"],
    );
    std::fs::write(e.work.path().join("mine.txt"), "keep").unwrap();
    let (code, v) = migrate(&e, &fake, &["--to", "steady"]);
    assert_eq!(code, 3, "{v}");
    assert_eq!(read(&e, "mine.txt"), "keep");
    assert_eq!(writes(&fake), 0);
}

#[test]
fn a_failed_run_deletes_only_what_it_created() {
    let fake = FakeGithub::start();
    fake.issue(3, "bosun registry", &["bosun:registry"], "");
    let e = env(&[
        (BOSUN_TOML, CONFIG),
        VERSION,
        (".gitignore", "scratch.txt\n"),
    ]);
    std::fs::write(e.work.path().join("scratch.txt"), "keep").unwrap();
    let hook = e.work.path().join(".git/hooks/pre-commit");
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    let (code, v) = migrate(&e, &fake, &["--to", "steady"]);
    assert_eq!(code, 1, "{v}");
    assert_eq!(read(&e, "scratch.txt"), "keep");
    assert!(!e.work.path().join(".bosun/registry").exists());
    assert_eq!(read(&e, BOSUN_TOML), CONFIG);
    assert_eq!(git(e.work.path(), &["status", "--porcelain"]), "");
}

#[test]
fn a_planted_push_refspec_cannot_retarget_the_push() {
    let fake = FakeGithub::start();
    let e = plain();
    let before = git(e.origin.path(), &["rev-parse", "refs/heads/main"]);
    git(
        e.work.path(),
        &["config", "remote.origin.push", "HEAD:refs/heads/main"],
    );
    let (code, v) = migrate(&e, &fake, &["--to", "steady"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        git(e.origin.path(), &["rev-parse", "refs/heads/main"]),
        before
    );
    let pushed = git(
        e.origin.path(),
        &["branch", "--list", "bosun/migrate-steady"],
    );
    assert!(pushed.contains("bosun/migrate-steady"), "{pushed}");
}
