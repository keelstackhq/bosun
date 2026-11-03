//! `bosun init --intake FILE` (zero-touch onboarding, spec section 3): the schema check first, the
//! flags derived from the file, the file copied to `.bosun/intake.toml` and left out of the
//! splice render, and the Routines' `create_trigger` calls in `next`.
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;

const OK_JSON: &str = r#"{"ok":true,"command":"channel add","changes":[{"path":".bosun/bosun.toml","action":"create","class":"shared"}],"messages":[]}"#;

const INTAKE: &str = r#"version = 1
repo = { name = "app", owner = "keelstackhq", visibility = "private" }
stack = ["rust"]
pieces = ["base", "rust-cli"]
bosun_pieces = ["claude-md", "circleci"]
goal = "A CLI that does a thing"
publish = ["crates"]
approvals = "orchestrator"
notify = "chat"
slack_channel = ""
"#;

const GOAL: &str = "goal = \"A CLI that does a thing\"";

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

struct Env {
    repo: tempfile::TempDir,
    tools: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        let (repo, origin, tools) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        git(origin.path(), &["init", "-q", "--bare", "-b", "main"]);
        git(repo.path(), &["init", "-q", "-b", "main"]);
        std::fs::write(repo.path().join("README.md"), "hi\n").unwrap();
        git(repo.path(), &["add", "-A"]);
        git(repo.path(), &["commit", "-q", "-m", "x"]);
        git(
            repo.path(),
            &["remote", "add", "origin", origin.path().to_str().unwrap()],
        );
        // The bare origin lives as long as the repo's remote: keep it in the tools dir's shadow.
        std::mem::forget(origin);
        Env { repo, tools }
    }

    /// A fake splice that logs its argv and writes the channel files under `-C`.
    fn splice(&self) -> PathBuf {
        let script = self.tools.path().join("splice");
        let body = format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{log}'\nwhile [ $# -gt 0 ]; do [ \"$1\" = -C ] && d=\"$2\"; shift; done\nmkdir -p \"$d/.splice/channels\" \"$d/.bosun\"\necho '# x' > \"$d/.splice/channels/bosun.toml\"\necho '# x' > \"$d/.bosun/bosun.toml\"\necho '{OK_JSON}'\n",
            log = self.log().display()
        );
        std::fs::write(&script, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    fn log(&self) -> PathBuf {
        self.tools.path().join("argv.log")
    }

    fn argv(&self) -> String {
        std::fs::read_to_string(self.log()).unwrap_or_default()
    }

    /// Write an intake file outside the repo and return its path.
    fn intake(&self, text: &str) -> String {
        let p = self.tools.path().join("intake.toml");
        std::fs::write(&p, text).unwrap();
        p.display().to_string()
    }

    fn run(&self, args: &[&str]) -> (i32, Value) {
        self.run_in(None, args)
    }

    fn run_in(&self, cwd: Option<&Path>, args: &[&str]) -> (i32, Value) {
        let fake = FakeGithub::start();
        let mut cmd = bosun();
        if let Some(c) = cwd {
            cmd.current_dir(c);
        }
        let out = cmd
            .env("BOSUN_GITHUB_API", fake.url())
            .env("BOSUN_SPLICE", self.splice())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("CLAUDE_CODE_REMOTE_ENVIRONMENT_TYPE")
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

    fn branches(&self) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(self.repo.path())
            .args(["branch", "--list"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// Nothing written: no branch, no splice call, no .bosun directory.
    fn untouched(&self) {
        assert_eq!(self.branches(), "* main");
        assert_eq!(self.argv(), "", "splice must not run");
        assert!(!self.repo.path().join(".bosun").exists());
    }
}

fn triggers(v: &Value) -> Vec<&Value> {
    v["next"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["tool"] == "create_trigger")
        .collect()
}

fn names(v: &Value) -> Vec<&str> {
    triggers(v)
        .iter()
        .map(|n| n["args"]["name"].as_str().unwrap())
        .collect()
}

#[test]
fn intake_derives_the_flags_copies_the_file_and_excludes_it() {
    let e = Env::new();
    let f = e.intake(INTAKE);
    let (code, v) = e.run(&["--intake", &f, "--no-pr"]);
    assert_eq!(code, 0, "{v}");
    let argv = e.argv();
    assert!(argv.contains("--data owner=keelstackhq"), "{argv}");
    assert!(argv.contains("--data project=app"), "{argv}");
    assert!(argv.contains("--data intake=1"), "{argv}");
    for p in ["rust", "claude-md", "circleci"] {
        assert!(argv.contains(&format!("--piece {p} ")), "{p}: {argv}");
    }
    assert_eq!(argv.matches("--piece claude-md ").count(), 1, "{argv}");
    assert!(argv.contains("--exclude .bosun/intake.toml"), "{argv}");
    assert_eq!(
        std::fs::read_to_string(e.repo.path().join(".bosun/intake.toml")).unwrap(),
        INTAKE
    );
    // Committed on the init branch with the rest.
    let shown = Command::new("git")
        .arg("-C")
        .arg(e.repo.path())
        .args(["show", "--stat", "--format=", "HEAD"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&shown.stdout).contains(".bosun/intake.toml"));
    assert_eq!(
        v["data"]["pieces"],
        serde_json::json!(["rust", "claude-md", "circleci"])
    );
    assert!(
        v["data"]["excluded"]
            .as_array()
            .unwrap()
            .contains(&".bosun/intake.toml".into())
    );
}

#[test]
fn intake_derives_the_plan() {
    let e = Env::new();
    std::fs::create_dir_all(e.repo.path().join("plans")).unwrap();
    std::fs::write(e.repo.path().join("plans/app.md"), "# plan\n").unwrap();
    git(e.repo.path(), &["add", "-A"]);
    git(e.repo.path(), &["commit", "-q", "-m", "plan"]);
    let f = e.intake(&INTAKE.replace(GOAL, "plan = \"plans/app.md\""));
    let (code, v) = e.run(&["--intake", &f, "--no-pr"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["plan"], "plans/app.md");
    assert_eq!(
        std::fs::read_to_string(e.repo.path().join("plans/app.md")).unwrap(),
        "# plan\n"
    );
}

#[test]
fn a_missing_answer_exits_5_and_writes_nothing() {
    let e = Env::new();
    let f = e.intake(&INTAKE.replace("approvals = \"orchestrator\"", "approvals = \"\""));
    let (code, v) = e.run(&["--intake", &f]);
    assert_eq!(code, 5, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("approvals"));
    e.untouched();
}

#[test]
fn a_bad_value_exits_2_and_writes_nothing() {
    let e = Env::new();
    let f = e.intake(&INTAKE.replace("\"private\"", "\"secret\""));
    let (code, v) = e.run(&["--intake", &f]);
    assert_eq!(code, 2, "{v}");
    e.untouched();
}

#[test]
fn a_flag_that_disagrees_with_the_file_exits_2_naming_both() {
    for (flag, val, file_val) in [
        ("--stack", "ts-pnpm", "rust"),
        ("--owner", "someone", "keelstackhq"),
        ("--plan", "plans/other.md", "plans/app.md"),
    ] {
        let e = Env::new();
        let text = if flag == "--plan" {
            INTAKE.replace(GOAL, "plan = \"plans/app.md\"")
        } else {
            INTAKE.to_string()
        };
        let f = e.intake(&text);
        let (code, v) = e.run(&["--intake", &f, flag, val]);
        assert_eq!(code, 2, "{flag}: {v}");
        let msg = v["error"]["message"].as_str().unwrap();
        assert!(msg.contains(val) && msg.contains(file_val), "{flag}: {msg}");
        e.untouched();
    }
}

#[test]
fn a_flag_that_agrees_with_the_file_is_fine_and_stack_is_optional() {
    let e = Env::new();
    let f = e.intake(INTAKE);
    let args = [
        "--intake",
        &f,
        "--stack",
        "rust",
        "--owner",
        "keelstackhq",
        "--no-pr",
    ];
    let (code, v) = e.run(&args);
    assert_eq!(code, 0, "{v}");
    let (code, _) = Env::new().run(&["--no-pr"]);
    assert_eq!(code, 2, "without --intake, --stack stays required");
}

#[test]
fn routines_are_create_trigger_calls_in_next() {
    let e = Env::new();
    let f = e.intake(INTAKE);
    let (code, v) = e.run(&["--intake", &f, "--no-pr"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(names(&v), ["janitor", "curator"], "no notify without slack");
    let t = triggers(&v);
    assert_eq!(t[0]["args"]["cron_expression"], "0 * * * *");
    assert_eq!(t[1]["args"]["create_new_session_on_fire"], true);
    for n in &t {
        assert!(n["args"]["prompt"].as_str().unwrap().contains("bosun"));
        assert!(n["args"]["initiation"].is_string());
    }
    // The prompts are the ones `bosun doctor` prints.
    let routines = v["data"]["routines"].as_array().unwrap();
    assert_eq!(routines[0]["model"], "haiku");
    assert_eq!(routines[1]["model"], "sonnet");
    for (r, n) in routines.iter().zip(&t) {
        let sent = n["args"]["prompt"].as_str().unwrap();
        let head = "Work in keelstackhq/app: attach it with add_repo and clone it first, then: ";
        assert_eq!(sent.strip_prefix(head), r["prompt"].as_str(), "{sent}");
    }
}

#[test]
fn notify_gets_a_routine_only_with_slack() {
    let e = Env::new();
    let f = e.intake(&INTAKE.replace(
        "notify = \"chat\"\nslack_channel = \"\"",
        "notify = \"slack\"\nslack_channel = \"C0123\"",
    ));
    let (code, v) = e.run(&["--intake", &f, "--no-pr"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(names(&v), ["janitor", "curator", "notify"]);
    let t = triggers(&v);
    let janitor = t[0]["args"]["prompt"].as_str().unwrap();
    assert!(janitor.contains("bosun notify"));
    assert!(
        t[2]["args"]["prompt"]
            .as_str()
            .unwrap()
            .contains("--channel-info")
    );
}

#[test]
fn only_the_steps_that_stay_human_are_listed() {
    let e = Env::new();
    let f = e.intake(INTAKE);
    let (_, v) = e.run(&["--intake", &f, "--no-pr"]);
    let all = v["data"]["owner_steps"].to_string();
    assert!(all.contains("setup PR"), "{all}");
    assert!(all.contains("weekly splice update"), "{all}");
    assert!(
        all.contains("hoist release bootstrap"),
        "publish = crates: {all}"
    );
    for gone in [
        "hoist apply",
        "bosun:halt",
        "SPLICE_BOT_TOKEN",
        "Routines",
        "owners.toml",
    ] {
        assert!(!all.contains(gone), "{gone} is automated: {all}");
    }
    let e = Env::new();
    let f = e.intake(&INTAKE.replace("[\"crates\"]", "[\"none\"]"));
    let (_, v) = e.run(&["--intake", &f, "--no-pr"]);
    assert!(!v["data"]["owner_steps"].to_string().contains("bootstrap"));
}

#[test]
fn without_intake_nothing_changes() {
    let e = Env::new();
    let (code, v) = e.run(&["--stack", "rust", "--no-pr"]);
    assert_eq!(code, 0, "{v}");
    assert!(v["next"].as_array().unwrap().is_empty());
    assert!(!e.argv().contains("intake"));
    assert!(v["data"]["owner_steps"].as_array().unwrap().len() >= 7);
}

fn symlink_case(link_dir: bool) {
    let e = Env::new();
    let outside = tempfile::tempdir().unwrap();
    let root = e.repo.path();
    if link_dir {
        std::os::unix::fs::symlink(outside.path(), root.join(".bosun")).unwrap();
    } else {
        std::fs::create_dir_all(root.join(".bosun")).unwrap();
        let target = outside.path().join("x");
        std::os::unix::fs::symlink(&target, root.join(".bosun/intake.toml")).unwrap();
    }
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "-m", "link"]);
    let f = e.intake(INTAKE);
    let (code, v) = e.run(&["--intake", &f, "--no-pr"]);
    assert_eq!(code, 1, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("symlink"));
    assert_eq!(e.branches(), "* main");
    assert_eq!(e.argv(), "", "splice must not run");
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn a_symlinked_bosun_dir_is_refused_and_nothing_leaves_the_repo() {
    symlink_case(true);
}

#[test]
fn a_symlinked_intake_file_is_refused_and_nothing_leaves_the_repo() {
    symlink_case(false);
}

#[test]
fn an_owner_with_an_underscore_is_a_bad_value_naming_repo_owner() {
    let e = Env::new();
    let f = e.intake(&INTAKE.replace("keelstackhq", "my_org"));
    let (code, v) = e.run(&["--intake", &f]);
    assert_eq!(code, 2, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("repo.owner"));
    e.untouched();
}

#[test]
fn intake_resolves_against_the_working_directory_and_plan_is_compared_canonically() {
    let e = Env::new();
    std::fs::create_dir_all(e.repo.path().join("plans")).unwrap();
    std::fs::write(e.repo.path().join("plans/app.md"), "# plan\n").unwrap();
    git(e.repo.path(), &["add", "-A"]);
    git(e.repo.path(), &["commit", "-q", "-m", "plan"]);
    // ./intake.toml sits in the cwd, not in the -C repo (which has none).
    let cwd = tempfile::tempdir().unwrap();
    let text = INTAKE.replace(GOAL, "plan = \"plans/app.md\"");
    std::fs::write(cwd.path().join("intake.toml"), &text).unwrap();
    // The same plan under another spelling agrees.
    let alias = format!("{}/plans/../plans/app.md", e.repo.path().display());
    let args = ["--intake", "./intake.toml", "--plan", &alias, "--no-pr"];
    let (code, v) = e.run_in(Some(cwd.path()), &args);
    assert_eq!(code, 0, "{v}");
    let copied = std::fs::read_to_string(e.repo.path().join(".bosun/intake.toml")).unwrap();
    assert_eq!(copied, text);
}
