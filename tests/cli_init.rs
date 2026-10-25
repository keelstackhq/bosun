mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;

const OK_JSON: &str = r#"{"ok":true,"command":"channel add","changes":[{"path":".bosun/bosun.toml","action":"create","class":"shared"}],"messages":[]}"#;
const FAIL_JSON: &str =
    r#"{"ok":false,"command":"channel add","changes":[],"messages":["no such piece","try again"]}"#;

fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .unwrap();
    assert!(st.success(), "git {args:?}");
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct Env {
    repo: tempfile::TempDir,
    origin: tempfile::TempDir,
    tools: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        let repo = tempfile::tempdir().unwrap();
        let origin = tempfile::tempdir().unwrap();
        let tools = tempfile::tempdir().unwrap();
        git(origin.path(), &["init", "-q", "--bare", "-b", "main"]);
        git(repo.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(repo.path().join("README.md"), "hi\n").unwrap();
        git(repo.path(), &["add", "-A"]);
        git(repo.path(), &["commit", "-q", "-m", "x"]);
        git(
            repo.path(),
            &["remote", "add", "origin", origin.path().to_str().unwrap()],
        );
        Env {
            repo,
            origin,
            tools,
        }
    }

    /// A fake splice: logs its argv, writes two files under `-C`, prints `json`, exits `code`.
    fn splice(&self, json: &str, code: i32) -> PathBuf {
        let script = self.tools.path().join("splice");
        let body = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log}'\nwhile [ $# -gt 0 ]; do [ \"$1\" = -C ] && d=\"$2\"; shift; done\nmkdir -p \"$d/.splice/channels\" \"$d/.bosun\"\necho '# x' > \"$d/.splice/channels/bosun.toml\"\necho '# x' > \"$d/.bosun/bosun.toml\"\necho '{json}'\nexit {code}\n",
            log = self.log().display()
        );
        std::fs::write(&script, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        script
    }

    fn log(&self) -> PathBuf {
        self.tools.path().join("argv.log")
    }

    fn argv(&self) -> String {
        std::fs::read_to_string(self.log()).unwrap_or_default()
    }

    fn run(&self, fake: &FakeGithub, splice: &Path, args: &[&str]) -> (i32, Value) {
        self.run_with(fake, splice, args, |_| {})
    }

    /// Like `run`, with no host git identity (bosun must commit anyway) and an env hook.
    fn run_with(
        &self,
        fake: &FakeGithub,
        splice: &Path,
        args: &[&str],
        env: impl FnOnce(&mut assert_cmd::Command),
    ) -> (i32, Value) {
        let mut cmd = bosun();
        cmd.env("BOSUN_GITHUB_API", fake.url())
            .env("BOSUN_SPLICE", splice)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1");
        env(&mut cmd);
        let out = cmd
            .arg("-C")
            .arg(self.repo.path())
            .arg("init")
            .args(args)
            .output()
            .unwrap();
        let v = one_object(&out.stdout);
        if v["ok"] == false {
            assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
        }
        (out.status.code().unwrap(), v)
    }

    fn origin_has_branch(&self) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(self.origin.path())
            .args(["rev-parse", "--verify", "-q", "refs/heads/bosun/init"])
            .status()
            .unwrap()
            .success()
    }
}

#[test]
fn happy_path_commits_pushes_and_opens_the_pr() {
    let e = Env::new();
    let fake = FakeGithub::start();
    let splice = e.splice(OK_JSON, 0);
    let plan = e.tools.path().join("my-plan.md");
    std::fs::write(&plan, "# plan\n").unwrap();
    let (code, v) = e.run(
        &fake,
        &splice,
        &[
            "--stack",
            "rust",
            "--owner",
            "octocat",
            "--plan",
            plan.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "init");
    let d = &v["data"];
    assert_eq!(d["branch"], "bosun/init");
    assert_eq!(d["risk"], "high");
    assert_eq!(d["plan"], "plans/my-plan.md");
    assert_eq!(d["pieces"][0], "rust");
    assert_eq!(d["changes"][0]["path"], ".bosun/bosun.toml");
    assert_eq!(d["owner_steps"].as_array().unwrap().len(), 6);
    assert!(d["pr"]["number"].as_u64().is_some());
    assert!(v["next"].as_array().unwrap().is_empty());
    assert!(e.repo.path().join("plans/my-plan.md").exists());
    assert!(e.origin_has_branch());
    let argv = e.argv();
    let version = env!("CARGO_PKG_VERSION");
    let want = format!("--as bosun --ref v{version} --piece rust");
    assert!(argv.contains(&want), "{argv}");
    assert!(argv.contains("--yes --json -C"), "{argv}");
    assert!(argv.contains("--data project="), "{argv}");
    assert!(argv.contains("--data owner=octocat"), "{argv}");
    assert!(!argv.contains("bosun_version"), "{argv}");
    let posts: Vec<_> = fake
        .requests()
        .into_iter()
        .filter(|r| r.method == "POST" && r.path.ends_with("/pulls"))
        .collect();
    assert_eq!(posts.len(), 1);
    let body = posts[0].body.as_deref().unwrap_or("");
    assert!(body.contains(r#""base":"main""#), "{body}");
    assert!(
        body.contains("R-gov") && body.contains("hoist apply"),
        "{body}"
    );
}

#[test]
fn bad_arguments_are_usage_errors() {
    let e = Env::new();
    let fake = FakeGithub::start();
    let splice = e.splice(OK_JSON, 0);
    for args in [
        vec![],
        vec!["--stack", "cobol"],
        vec!["--stack", "rust", "--stack", "rust"],
        vec!["--stack", "rust", "--bosun-version", "1.2"],
        vec!["--stack", "rust", "--plan", "/no/such/plan.md"],
        vec!["--stack", "rust", "--owner", "a b"],
        vec!["--stack", "rust", "--source=-x"],
    ] {
        let (code, v) = e.run(&fake, &splice, &args);
        assert_eq!(code, 2, "{args:?}: {v}");
    }
    assert!(e.argv().is_empty());
}

#[test]
fn a_splice_failure_is_an_error_with_its_messages() {
    let e = Env::new();
    let fake = FakeGithub::start();
    let splice = e.splice(FAIL_JSON, 3);
    let (code, v) = e.run(&fake, &splice, &["--stack", "rust"]);
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no such piece")
    );
    assert_eq!(v["data"]["splice"]["ok"], false);
    // Rolled back: on main, clean, no init branch, so a rerun works.
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(
        fix.contains("rolled back to main") && !fix.contains("git switch -"),
        "{fix}"
    );
    assert_eq!(git_out(e.repo.path(), &["status", "--porcelain"]), "");
    assert_eq!(
        git_out(e.repo.path(), &["branch", "--show-current"]),
        "main"
    );
    let ok = e.splice(OK_JSON, 0);
    let (code, v) = e.run(&fake, &ok, &["--stack", "rust", "--no-pr"]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn no_token_fails_before_anything_changes() {
    let e = Env::new();
    let fake = FakeGithub::start();
    let splice = e.splice(OK_JSON, 0);
    let (code, v) = e.run_with(&fake, &splice, &["--stack", "rust"], |c| {
        c.env_remove("GITHUB_TOKEN");
    });
    assert_eq!(code, 1, "{v}");
    assert!(e.argv().is_empty());
    assert_eq!(
        git_out(e.repo.path(), &["branch", "--show-current"]),
        "main"
    );
    assert!(!e.origin_has_branch());
}

#[test]
fn a_plan_already_in_plans_is_kept() {
    let e = Env::new();
    std::fs::create_dir_all(e.repo.path().join("plans")).unwrap();
    std::fs::write(e.repo.path().join("plans/p.md"), "# plan\n").unwrap();
    git(e.repo.path(), &["add", "-A"]);
    git(e.repo.path(), &["commit", "-q", "-m", "plan"]);
    let fake = FakeGithub::start();
    let splice = e.splice(OK_JSON, 0);
    let plan = e.repo.path().join("plans/p.md");
    let args = [
        "--stack",
        "rust",
        "--no-pr",
        "--plan",
        plan.to_str().unwrap(),
    ];
    let (code, v) = e.run(&fake, &splice, &args);
    assert_eq!(code, 0, "{v}");
    assert_eq!(std::fs::read_to_string(&plan).unwrap(), "# plan\n");
}

/// A branch ref has no release version, so init names the pin; a repo's own settings file is
/// kept; `--no-pr` commits locally with no push and no GitHub call.
#[test]
fn no_pr_with_a_branch_ref_keeps_the_repo_settings() {
    let e = Env::new();
    std::fs::create_dir_all(e.repo.path().join(".claude")).unwrap();
    std::fs::write(e.repo.path().join(".claude/settings.json"), "{}").unwrap();
    git(e.repo.path(), &["add", "-A"]);
    git(e.repo.path(), &["commit", "-q", "-m", "settings"]);
    let fake = FakeGithub::start();
    let splice = e.splice(OK_JSON, 0);
    let args = ["--stack", "rust", "--ref", "main", "--no-pr"];
    let (code, v) = e.run(&fake, &splice, &args);
    assert_eq!(code, 0, "{v}");
    let argv = e.argv();
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        argv.contains(&format!("--data bosun_version={version}")),
        "{argv}"
    );
    assert!(argv.contains("--ref main"), "{argv}");
    assert!(argv.contains("--exclude .claude/settings.json"), "{argv}");
    assert!(
        !argv.contains("--exclude .github/workflows/splice.yml"),
        "{argv}"
    );
    assert_eq!(v["data"]["excluded"][0], ".claude/settings.json");
    assert!(v["data"]["pr"].is_null());
    assert!(v["data"]["commit"].as_str().unwrap().len() >= 7);
    assert!(fake.requests().is_empty());
    assert!(!e.origin_has_branch());
}

/// An initialised repo is a conflict (3) and a dirty tree an error (1); neither runs splice.
#[test]
fn refusals_run_no_splice() {
    let e = Env::new();
    std::fs::write(e.repo.path().join("dirty.txt"), "x").unwrap();
    let fake = FakeGithub::start();
    let splice = e.splice(OK_JSON, 0);
    let (code, v) = e.run(&fake, &splice, &["--stack", "rust"]);
    assert_eq!(code, 1, "{v}");
    std::fs::create_dir_all(e.repo.path().join(".splice/channels")).unwrap();
    std::fs::write(e.repo.path().join(".splice/channels/bosun.toml"), "x").unwrap();
    let (code, v) = e.run(&fake, &splice, &["--stack", "rust"]);
    assert_eq!(code, 3, "{v}");
    assert!(e.argv().is_empty());
}

/// A plain repo: the claude-md piece is added, there is no own_ci, and the seeded hoist.toml
/// (written by splice) is left alone.
#[test]
fn a_plain_repo_gets_claude_md_and_no_own_ci() {
    let e = Env::new();
    let fake = FakeGithub::start();
    let splice = e.splice(OK_JSON, 0);
    let (code, v) = e.run(&fake, &splice, &["--stack", "rust", "--no-pr"]);
    assert_eq!(code, 0, "{v}");
    let argv = e.argv();
    assert!(argv.contains("--piece rust --piece claude-md"), "{argv}");
    assert!(!argv.contains("own_ci"), "{argv}");
    assert!(!argv.contains("--exclude"), "{argv}");
    assert_eq!(v["data"]["own_ci"], false);
    assert_eq!(v["data"]["hoist_checks_added"].as_array().unwrap().len(), 0);
    assert_eq!(v["data"]["owner_steps"].as_array().unwrap().len(), 6);
}

fn commit_file(e: &Env, path: &str, body: &str) {
    let p = e.repo.path().join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

const HOIST: &str = "# keep me\n[github]\n# the gates\nrequired_checks = [\"ci-ok\"] # trailing\nauto_merge = true\n";

/// A hull template-rust repo: it owns ci.yml, CLAUDE.md and settings.json, and has a hoist.toml.
#[test]
fn a_hull_repo_gets_own_ci_excludes_and_hoist_checks() {
    let e = Env::new();
    commit_file(
        &e,
        ".github/workflows/ci.yml",
        "name: ci\njobs:\n  build:\n    runs-on: x\n  ci-ok: # gate\n    needs: [build]\n",
    );
    commit_file(&e, ".claude/settings.json", "{}");
    commit_file(&e, "CLAUDE.md", "# x\n");
    commit_file(&e, "hoist.toml", HOIST);
    commit_file(
        &e,
        ".splice/lock.toml",
        "[files]\n\"CLAUDE.md\" = \"a\"\n\".claude/settings.json\" = \"b\"\n\".github/workflows/ci.yml\" = \"c\"\n",
    );
    git(e.repo.path(), &["add", "-A"]);
    git(e.repo.path(), &["commit", "-q", "-m", "hull"]);
    let fake = FakeGithub::start();
    let splice = e.splice(OK_JSON, 0);
    let (code, v) = e.run(&fake, &splice, &["--stack", "rust"]);
    assert_eq!(code, 0, "{v}");
    let argv = e.argv();
    assert!(argv.contains("--data own_ci=1"), "{argv}");
    assert!(!argv.contains("claude-md"), "{argv}");
    assert!(argv.contains("--exclude hoist.toml"), "{argv}");
    assert!(argv.contains("--exclude .claude/settings.json"), "{argv}");
    let hoist = std::fs::read_to_string(e.repo.path().join("hoist.toml")).unwrap();
    assert_eq!(
        hoist,
        HOIST.replace(
            "[\"ci-ok\"]",
            "[\"ci-ok\", \"risk-check\", \"review-verdict\", \"main-open\", \"bosun-ok\"]"
        )
    );
    let d = &v["data"];
    assert_eq!(d["own_ci"], true);
    assert_eq!(
        d["hoist_checks_added"],
        serde_json::json!(["risk-check", "review-verdict", "main-open", "bosun-ok"])
    );
    let steps = d["owner_steps"].as_array().unwrap();
    assert!(steps.iter().any(|s| {
        s.as_str()
            .unwrap()
            .contains("hooks are missing from .claude/settings.json")
    }));
    let posts: Vec<_> = fake
        .requests()
        .into_iter()
        .filter(|r| r.path.ends_with("/pulls"))
        .collect();
    assert!(
        posts[0]
            .body
            .as_deref()
            .unwrap()
            .contains("hooks are missing from .claude/settings.json")
    );
}

/// A settings.json that already runs both bosun hooks needs no owner step.
#[test]
fn settings_with_the_hooks_need_no_owner_step() {
    let e = Env::new();
    commit_file(
        &e,
        ".claude/settings.json",
        "{\"cmd\":[\"bosun hook pre-tool-use\",\"bosun hook session-start\"]}",
    );
    git(e.repo.path(), &["add", "-A"]);
    git(e.repo.path(), &["commit", "-q", "-m", "hooks"]);
    let fake = FakeGithub::start();
    let splice = e.splice(OK_JSON, 0);
    let (code, v) = e.run(&fake, &splice, &["--stack", "rust", "--no-pr"]);
    assert_eq!(code, 0, "{v}");
    assert!(e.argv().contains("--exclude .claude/settings.json"));
    assert_eq!(v["data"]["owner_steps"].as_array().unwrap().len(), 6);
}

/// An invalid hoist.toml fails before splice runs, so there is nothing to roll back.
#[test]
fn an_invalid_hoist_toml_fails_before_splice() {
    let e = Env::new();
    commit_file(&e, "hoist.toml", "[github\n");
    git(e.repo.path(), &["add", "-A"]);
    git(e.repo.path(), &["commit", "-q", "-m", "bad"]);
    let fake = FakeGithub::start();
    let splice = e.splice(OK_JSON, 0);
    let (code, v) = e.run(&fake, &splice, &["--stack", "rust", "--no-pr"]);
    assert_eq!(code, 1, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("valid TOML"));
    assert!(e.argv().is_empty());
    assert_eq!(git_out(e.repo.path(), &["status", "--porcelain"]), "");
    assert_eq!(
        git_out(e.repo.path(), &["branch", "--show-current"]),
        "main"
    );
}

/// own_ci means a `ci-ok` job exists in some workflow, not that ci.yml exists.
#[test]
fn own_ci_needs_a_ci_ok_job() {
    let e = Env::new();
    commit_file(
        &e,
        ".github/workflows/ci.yml",
        "jobs:\n  build:\n    runs-on: x\n",
    );
    git(e.repo.path(), &["add", "-A"]);
    git(e.repo.path(), &["commit", "-q", "-m", "ci"]);
    let fake = FakeGithub::start();
    let splice = e.splice(OK_JSON, 0);
    let (code, v) = e.run(&fake, &splice, &["--stack", "rust", "--no-pr"]);
    assert_eq!(code, 0, "{v}");
    assert!(!e.argv().contains("own_ci"), "{}", e.argv());
    assert_eq!(v["data"]["own_ci"], false);
}

#[test]
fn a_ci_ok_job_in_another_workflow_sets_own_ci() {
    let e = Env::new();
    commit_file(
        &e,
        ".github/workflows/build.yaml",
        "on: push\njobs:\n  ci-ok:\n    runs-on: x\n",
    );
    git(e.repo.path(), &["add", "-A"]);
    git(e.repo.path(), &["commit", "-q", "-m", "ci"]);
    let fake = FakeGithub::start();
    let splice = e.splice(OK_JSON, 0);
    let (code, v) = e.run(&fake, &splice, &["--stack", "rust", "--no-pr"]);
    assert_eq!(code, 0, "{v}");
    assert!(e.argv().contains("--data own_ci=1"), "{}", e.argv());
    assert_eq!(v["data"]["own_ci"], true);
    let step = v["data"]["owner_steps"][1].as_str().unwrap();
    assert!(step.contains("bosun-ok"), "{step}");
}

/// own_ci follows the check context, as GitHub names it: a job's `name:`, else its key, at any
/// indentation (the parse of `core::checks::workflow_checks`).
#[test]
fn own_ci_reads_the_check_context_not_the_job_key() {
    for (yaml, own) in [
        (
            "on: push\njobs:\n    gate:\n        name: ci-ok\n        runs-on: x\n",
            true,
        ),
        (
            "on: push\njobs:\n  ci-ok:\n    name: all green\n    runs-on: x\n",
            false,
        ),
    ] {
        let e = Env::new();
        commit_file(&e, ".github/workflows/ci.yml", yaml);
        git(e.repo.path(), &["add", "-A"]);
        git(e.repo.path(), &["commit", "-q", "-m", "ci"]);
        let fake = FakeGithub::start();
        let splice = e.splice(OK_JSON, 0);
        let (code, v) = e.run(&fake, &splice, &["--stack", "rust", "--no-pr"]);
        assert_eq!(code, 0, "{v}");
        assert_eq!(v["data"]["own_ci"], own, "{yaml}");
        assert_eq!(e.argv().contains("--data own_ci=1"), own, "{}", e.argv());
    }
}

/// A channel lock's [files] counts as ownership of CLAUDE.md too; the piece is then left out.
#[test]
fn a_channel_lock_owning_claude_md_skips_the_piece() {
    let e = Env::new();
    commit_file(
        &e,
        ".splice/channels/other.toml",
        "[files]\n\"CLAUDE.md\" = \"x\"\n",
    );
    git(e.repo.path(), &["add", "-A"]);
    git(e.repo.path(), &["commit", "-q", "-m", "ch"]);
    let fake = FakeGithub::start();
    let splice = e.splice(OK_JSON, 0);
    let (code, v) = e.run(&fake, &splice, &["--stack", "rust", "--no-pr"]);
    assert_eq!(code, 0, "{v}");
    assert!(!e.argv().contains("claude-md"), "{}", e.argv());
    assert_eq!(v["data"]["pieces"].as_array().unwrap().len(), 1);
}

/// A form init cannot edit is refused up front, and the fix names the checks to add by hand.
#[test]
fn an_uneditable_hoist_toml_names_the_checks_to_add() {
    let e = Env::new();
    commit_file(
        &e,
        "hoist.toml",
        "[ github ]\nrequired_checks = [\"ci-ok\"]\n",
    );
    git(e.repo.path(), &["add", "-A"]);
    git(e.repo.path(), &["commit", "-q", "-m", "odd"]);
    let fake = FakeGithub::start();
    let splice = e.splice(OK_JSON, 0);
    let (code, v) = e.run(&fake, &splice, &["--stack", "rust", "--no-pr"]);
    assert_eq!(code, 1, "{v}");
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(
        fix.contains("risk-check") && fix.contains("by hand"),
        "{fix}"
    );
    assert!(e.argv().is_empty());
}

/// The push worked but the PR did not: the fix names a command that opens it.
#[test]
fn a_failed_pr_after_the_push_names_the_command() {
    let e = Env::new();
    let fake = FakeGithub::start();
    fake.fail_path("/pulls", 422);
    let splice = e.splice(OK_JSON, 0);
    let (code, v) = e.run(&fake, &splice, &["--stack", "rust"]);
    assert_eq!(code, 1, "{v}");
    assert!(e.origin_has_branch());
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(
        fix.contains("gh pr create --head bosun/init --base main"),
        "{fix}"
    );
}
