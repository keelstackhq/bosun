//! `bosun intake check` (the intake file, spec section 3) and the old
//! `bosun intake <text>` form next to them.
mod common;

use common::fake::FakeGithub;
use serde_json::Value;
use std::path::{Path, PathBuf};

const NOW: &str = "2026-11-02T10:00:00Z";
const OK: &str = r#"version = 1
repo = { name = "app", owner = "keelstackhq", visibility = "private" }
stack = ["rust"]
pieces = ["base", "rust-cli"]
bosun_pieces = ["claude-md"]
goal = "A CLI that does a thing"
publish = ["crates"]
approvals = "orchestrator"
notify = "chat"
slack_channel = ""
"#;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Env {
        let e = Env {
            dir: tempfile::tempdir().unwrap(),
        };
        e.write(".bosun/intake.toml", OK);
        e
    }
    fn p(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }
    fn write(&self, rel: &str, text: &str) {
        let p = self.p(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    /// Run `bosun intake <args>` in the dir at `now`; asserts one object and a fix on errors.
    fn run(&self, now: &str, args: &[&str], stdin: &str) -> (i32, Value) {
        let out = common::bosun()
            .current_dir(self.dir.path())
            .env("BOSUN_NOW", now)
            .arg("intake")
            .args(args)
            .write_stdin(stdin.to_string())
            .output()
            .unwrap();
        let v = common::one_object(&out.stdout);
        if v["ok"] == false {
            assert!(
                !v["error"]["fix"].as_str().unwrap().trim().is_empty(),
                "{v}"
            );
        }
        (out.status.code().unwrap(), v)
    }
}

#[test]
fn check_validates_the_schema_and_reports_the_fields() {
    let e = Env::new();
    let (code, v) = e.run(NOW, &["check"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "intake check");
    assert_eq!(v["data"]["repo"]["name"], "app");
    assert_eq!(v["data"]["stack"][0], "rust");
    let (code, v) = e.run(NOW, &["check", "--from", "-"], OK);
    assert_eq!(code, 0, "{v}");
    let (code, v) = e.run(NOW, &["check", "nope.toml"], "");
    assert_eq!(code, 2, "{v}");
}

#[test]
fn check_separates_a_missing_answer_from_a_bad_value() {
    let e = Env::new();
    let cases = [
        ("stack = [\"rust\"]\n", "", 5, "stack"),
        (
            "goal = \"A CLI that does a thing\"",
            "goal = \"\"",
            5,
            "goal",
        ),
        ("\"private\"", "\"secret\"", 2, "repo.visibility"),
        ("[\"crates\"]", "[\"crates\", \"none\"]", 2, "publish"),
        ("\"chat\"", "\"slack\"", 5, "slack_channel"),
        ("goal = ", "plan = \"plans/a.md\"\ngoal = ", 2, "goal"),
        ("version = 1\n", "version = 1\nsurprise = 1\n", 2, "(file)"),
        (
            "visibility = \"private\"",
            "visibility = \"private\", x = 1",
            2,
            "(file)",
        ),
        ("stack = [\"rust\"]", "stack = \"rust\"", 2, "stack"),
    ];
    for (from, to, want, field) in cases {
        let (code, v) = e.run(NOW, &["check", "--from", "-"], &OK.replacen(from, to, 1));
        assert_eq!(code, want, "{field}: {v}");
        let err = &v["error"];
        let text = format!("{} {}", err["message"], err["fix"]);
        assert!(text.contains(field), "{field}: {v}");
        assert_eq!(v["data"]["field"], field);
    }
    let (code, v) = e.run(NOW, &["check", "--from", "-"], "version = [");
    assert_eq!(code, 2, "{v}");
}

#[test]
fn check_ignores_signatures_and_a_signature_table_is_an_unknown_key() {
    let e = Env::new();
    e.write(".bosun/intake.sig", "garbage");
    let (code, v) = e.run(NOW, &["check"], "");
    assert_eq!(code, 0, "{v}");
    assert!(v["data"].get("signed").is_none());
    let with_table = format!("{OK}\n[signature]\nkey = \"k\"\nsig = \"s\"\n");
    let (code, v) = e.run(NOW, &["check", "--from", "-"], &with_table);
    assert_eq!(code, 2, "{v}");
}

// UC-ZI-7: the old form keeps working.
#[test]
fn the_old_intake_text_form_still_opens_a_plan_issue() {
    let fake = FakeGithub::start();
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("plans")).unwrap();
    std::fs::write(
        d.path().join("plans/widget.md"),
        "# Plan: widget\n\nproject: widget\n\n## M1: Lifecycle\n\narea: lifecycle\n\n### Tasks\n\n- T1: State machine\n  touch: src/core/state.rs\n\n### Done when\n\n- UC-M1-1: a task moves planned -> merged.\n",
    )
    .unwrap();
    let dir: &Path = d.path();
    let args = [
        "-C",
        dir.to_str().unwrap(),
        "intake",
        "add a flag",
        "--source",
        "human",
    ];
    let (code, v) = common::run(&fake, "session_test", NOW, &args);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "intake");
    assert_eq!(v["data"]["created"], true);
    let issue = fake.issue_state(v["data"]["issue"].as_u64().unwrap());
    assert!(issue.labels.contains(&"bosun:source/human".to_string()));
    // The source is still required.
    let (code, v) = common::run(&fake, "session_test", NOW, &["intake", "x"]);
    assert_eq!(code, 2, "{v}");
}
