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
        let out = bosun()
            .env("BOSUN_GITHUB_API", fake.url())
            .env("BOSUN_SPLICE", splice)
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
    assert!(body.contains("bosun/init"), "{body}");
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
