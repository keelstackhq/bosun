//! Monorepo scoping (spec §3 `[[folders]]`, §8, §9): bosun's own protected paths stay gated
//! under its folder, and main-open blocks only PRs touching a red folder or one depending on it.
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::{Value, json};

const NOW: &str = "2026-11-20T10:00:00Z";
const BASE: &str = "base0000";
const MAIN: &str = "refs/heads/main";
const FOLDERS: &str = "[[folders]]\npath = \"bosun\"\n\n[[folders]]\npath = \"splice\"\ndepends_on = [\"bosun\"]\n\n[[folders]]\npath = \"hoist\"\n";

fn plus(line: &str) -> String {
    format!("@@ -0,0 +1,1 @@\n+{line}")
}

/// PR 1 on a monorepo base where bosun's code lives in `bosun/`.
fn mono_pr(path: &str) -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "Closes #7");
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.content(".bosun/bosun.toml", BASE, FOLDERS);
    fake.content("bosun/src/cli/risk_check.rs", BASE, "// bosun");
    fake.pull_file(1, path, "modified", 1, 0, Some(&plus("let x = 1;")));
    fake
}

fn risk(fake: &FakeGithub) -> (i32, Value) {
    run(fake, "session_test", NOW, &["risk-check", "1"])
}

fn rules(v: &Value) -> Vec<String> {
    (v["data"]["matches"].as_array().unwrap().iter())
        .map(|m| m["rule"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn bosun_gate_code_in_its_folder_is_still_owner_class() {
    for (path, rule) in [
        ("bosun/src/core/risk.rs", "R-gate"),
        ("bosun/src/cli/risk_check.rs", "R-gate"),
        ("bosun/.bosun/risk.toml", "R-gov"),
        (".bosun/bosun.toml", "R-gov"),
        (".github/workflows/ci.yml", "R-ci"),
    ] {
        let fake = mono_pr(path);
        let (code, v) = risk(&fake);
        assert_ne!(code, 0, "{path}: {v}");
        assert_eq!(v["data"]["level"], "high", "{path}: {v}");
        assert!(rules(&v).contains(&rule.to_string()), "{path}: {v}");
    }
}

#[test]
fn bosun_folder_is_protected_without_a_folders_table() {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "Closes #7");
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.content("bosun/src/cli/risk_check.rs", BASE, "// bosun");
    fake.pull_file(
        1,
        "bosun/src/core/risk.rs",
        "modified",
        1,
        0,
        Some(&plus("x")),
    );
    let (_, v) = risk(&fake);
    assert_eq!(v["data"]["level"], "high", "{v}");
}

#[test]
fn folder_paths_survive_a_base_config_the_release_rejects() {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "Closes #7");
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    let cfg = "future_key = 1\n[[folders]]\npath = \"tools/bosun\"\n";
    fake.content(".bosun/bosun.toml", BASE, cfg);
    fake.content("tools/bosun/src/cli/risk_check.rs", BASE, "// bosun");
    fake.pull_file(
        1,
        "tools/bosun/src/core/risk.rs",
        "modified",
        1,
        0,
        Some(&plus("x")),
    );
    let (_, v) = risk(&fake);
    assert_eq!(v["data"]["level"], "high", "{v}");
}

#[test]
fn other_folders_code_is_normal_risk() {
    let fake = mono_pr("splice/src/render.rs");
    let (code, v) = risk(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["level"], "normal");
}

fn mono_main(fake: &FakeGithub, runs: &[(&str, &str)]) {
    fake.issue(3, "main is red", &["bosun:main-red"], "ci-ok failed");
    fake.content(".bosun/bosun.toml", MAIN, FOLDERS);
    for (name, conclusion) in runs {
        fake.check_run("main", name, "completed", Some(conclusion));
    }
}

fn main_open(fake: &FakeGithub, pr: u64, files: &[&str]) -> (i32, Value) {
    fake.pull(pr, "feature", &format!("f{pr}"), "No link.");
    fake.pull_set(pr, "changed_files", json!(files.len()));
    for f in files {
        fake.pull_file(pr, f, "modified", 1, 0, Some("@@ -1 +1 @@\n+x"));
    }
    run(
        fake,
        "session_a",
        NOW,
        &["gate", "main-open", "--pr", &pr.to_string()],
    )
}

#[test]
fn red_folder_blocks_itself_and_its_dependents_only() {
    let fake = FakeGithub::start();
    mono_main(
        &fake,
        &[
            ("ci-ok", "failure"),
            ("ci-bosun", "failure"),
            ("ci-splice", "success"),
            ("ci-hoist", "success"),
        ],
    );
    let (code, v) = main_open(&fake, 10, &["hoist/src/a.rs"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["red_folders"], json!(["bosun"]));
    assert_eq!(v["data"]["blocked_touched"], json!([]));
    let (code, v) = main_open(&fake, 11, &["bosun/src/a.rs"]);
    assert_eq!(code, 4, "{v}");
    assert_eq!(v["data"]["blocked_touched"], json!(["bosun"]));
    // splice depends on bosun: blocked too.
    let (code, v) = main_open(&fake, 12, &["splice/src/a.rs"]);
    assert_eq!(code, 4, "{v}");
    assert_eq!(v["data"]["blocked_touched"], json!(["splice"]));
    // Root files touch every folder.
    for (pr, f) in [
        (13, "Cargo.lock"),
        (14, ".github/workflows/ci.yml"),
        (15, "deny.toml"),
    ] {
        let (code, v) = main_open(&fake, pr, &["hoist/a", f]);
        assert_eq!(code, 4, "{f}: {v}");
    }
}

#[test]
fn missing_or_foreign_folder_checks_fail_closed() {
    let fake = FakeGithub::start();
    // No ci-hoist run, and a green ci-splice from another App: both count red.
    mono_main(&fake, &[("ci-bosun", "success")]);
    fake.app_run("main", "ci-splice", "success", "ok", "evil-app", 99);
    let (code, v) = main_open(&fake, 10, &["hoist/src/a.rs"]);
    assert_eq!(code, 4, "{v}");
    let (code, v) = main_open(&fake, 11, &["splice/src/a.rs"]);
    assert_eq!(code, 4, "{v}");
    let (code, v) = main_open(&fake, 12, &["bosun/src/a.rs"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn red_main_with_every_folder_green_blocks_everything() {
    let fake = FakeGithub::start();
    // ci-ok failed on a root job: no folder check is red.
    mono_main(
        &fake,
        &[
            ("ci-ok", "failure"),
            ("ci-bosun", "success"),
            ("ci-splice", "success"),
            ("ci-hoist", "skipped"),
        ],
    );
    let (code, v) = main_open(&fake, 10, &["hoist/src/a.rs"]);
    assert_eq!(code, 4, "{v}");
}

#[test]
fn truncated_file_list_touches_every_folder() {
    let fake = FakeGithub::start();
    mono_main(
        &fake,
        &[
            ("ci-bosun", "failure"),
            ("ci-splice", "success"),
            ("ci-hoist", "success"),
        ],
    );
    fake.pull(10, "feature", "f10", "No link.");
    fake.pull_set(10, "changed_files", json!(5000));
    fake.pull_file(10, "hoist/a.rs", "modified", 1, 0, Some("@@ -1 +1 @@\n+x"));
    let (code, v) = run(
        &fake,
        "session_a",
        NOW,
        &["gate", "main-open", "--pr", "10"],
    );
    assert_eq!(code, 4, "{v}");
}

#[test]
fn main_health_rejudges_per_folder() {
    let fake = FakeGithub::start();
    fake.content(".bosun/bosun.toml", MAIN, FOLDERS);
    fake.check_run("main", "ci-bosun", "completed", Some("failure"));
    fake.check_run("main", "ci-splice", "completed", Some("success"));
    fake.check_run("main", "ci-hoist", "completed", Some("success"));
    fake.pull(10, "feature", "f10", "No link.");
    fake.pull_set(10, "changed_files", json!(1));
    fake.pull_file(10, "hoist/a.rs", "modified", 1, 0, Some("@@ -1 +1 @@\n+x"));
    fake.pull(11, "feature", "f11", "No link.");
    fake.pull_set(11, "changed_files", json!(1));
    fake.pull_file(11, "bosun/a.rs", "modified", 1, 0, Some("@@ -1 +1 @@\n+x"));
    let (code, v) = run(
        &fake,
        "session_a",
        NOW,
        &[
            "main-health",
            "--result",
            "failure",
            "--run-url",
            "https://x/run/1",
        ],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["red_folders"], json!(["bosun"]));
    let last = |sha: &str| fake.statuses_of(sha).last().unwrap()["state"].clone();
    assert_eq!(last("sha10"), "success");
    assert_eq!(last("sha11"), "failure");
}
