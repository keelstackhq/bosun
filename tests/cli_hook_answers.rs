mod common;

use std::path::Path;
use std::process::Command;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

const WAITING: &str = "high risk, owner approval needed: R-gov";
const NOW: &str = "2026-10-22T10:00:00Z";

fn head(prefix: &str) -> String {
    format!("{prefix:0<40}")
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn repo() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    git(tmp.path(), &["init", "-q", "-b", "main"]);
    git(
        tmp.path(),
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/acme/widget.git",
        ],
    );
    tmp
}

fn answers_file(repo: &Path) -> std::path::PathBuf {
    let common = git(repo, &["rev-parse", "--git-common-dir"]);
    repo.join(common).join("bosun").join("answers.json")
}

fn stored(repo: &Path) -> Value {
    let raw = std::fs::read_to_string(answers_file(repo)).unwrap_or_else(|_| "{}".into());
    serde_json::from_str(&raw).unwrap()
}

/// A question as `approve --list` asks it, for (pr, head) pairs, with `answer` as the tool result.
fn question(items: &[(u64, &str)], answer: &str) -> Value {
    let line = |pr: u64, h: &str| {
        format!(
            "acme/widget#{pr} `t` (risk R-gov; head {s} = {h}; verdict: ok) [bosun:{pr}-{s}-1792663200]",
            s = &h[..7]
        )
    };
    let text = match items {
        [(pr, h)] => format!("Approve {}?", line(*pr, h)),
        many => {
            let lines: Vec<String> = many
                .iter()
                .map(|(p, h)| format!("- {}", line(*p, h)))
                .collect();
            format!(
                "Approve which PRs? Hold approves none.\n{}",
                lines.join("\n")
            )
        }
    };
    let mut options: Vec<Value> = items
        .iter()
        .map(|(p, h)| json!({"label": format!("Approve #{p} {}", &h[..7]), "description": "x"}))
        .collect();
    options.push(json!({"label": "Hold", "description": "x"}));
    let q = json!({"question": text, "header": "Approve", "multiSelect": items.len() > 1,
        "options": options});
    json!({"session_id": "s", "hook_event_name": "PostToolUse", "tool_name": "AskUserQuestion",
        "tool_input": {"questions": [q]},
        "tool_response": {"questions": [q], "answers": {text: answer}}})
}

fn post(repo: &Path, input: &Value) -> std::process::Output {
    bosun()
        .current_dir(repo)
        .env("BOSUN_NOW", NOW)
        .args(["hook", "post-tool-use"])
        .write_stdin(input.to_string())
        .output()
        .unwrap()
}

#[test]
fn answer_approve_stored() {
    let r = repo();
    let h = head("abc1234");
    post(r.path(), &question(&[(12, &h)], "Approve #12 abc1234"));
    let a = &stored(r.path())["12-abc1234-1792663200"];
    assert_eq!(a["answer"], json!("approve"), "{a}");
    assert_eq!(a["pr"], json!(12));
    assert_eq!(a["head"], json!(h));
    assert_eq!(a["at"], json!(NOW));
}

#[test]
fn answer_hold_stored() {
    let r = repo();
    let h = head("abc1234");
    post(r.path(), &question(&[(12, &h)], "Hold"));
    assert_eq!(
        stored(r.path())["12-abc1234-1792663200"]["answer"],
        json!("hold")
    );
}

#[test]
fn multiselect_approve_some_and_hold_wins() {
    let r = repo();
    let (a, b, c) = (head("aaaaaaa"), head("bbbbbbb"), head("ccccccc"));
    post(
        r.path(),
        &question(
            &[(1, &a), (2, &b), (3, &c)],
            "Approve #1 aaaaaaa, Approve #3 ccccccc",
        ),
    );
    let s = stored(r.path());
    assert_eq!(s["1-aaaaaaa-1792663200"]["answer"], json!("approve"));
    assert!(s.get("2-bbbbbbb-1792663200").is_none(), "{s}");
    assert_eq!(s["3-ccccccc-1792663200"]["answer"], json!("approve"));
    // Hold picked with an approve: Hold wins for every PR of the question.
    post(
        r.path(),
        &question(&[(1, &a), (2, &b)], "Approve #1 aaaaaaa, Hold"),
    );
    let s = stored(r.path());
    assert_eq!(s["1-aaaaaaa-1792663200"]["answer"], json!("hold"), "{s}");
    assert_eq!(s["2-bbbbbbb-1792663200"]["answer"], json!("hold"));
    assert_eq!(s["3-ccccccc-1792663200"]["answer"], json!("approve"));
}

#[test]
fn non_bosun_question_ignored() {
    let r = repo();
    let q = json!({"question": "Pick a colour?", "header": "Colour", "multiSelect": false,
        "options": [{"label": "Hold", "description": "x"}, {"label": "Red", "description": "y"}]});
    let input = json!({"tool_name": "AskUserQuestion", "tool_input": {"questions": [q]},
        "tool_response": {"answers": {"Pick a colour?": "Hold"}}});
    let out = post(r.path(), &input);
    assert_eq!(out.status.code(), Some(0));
    assert!(!answers_file(r.path()).exists());
}

#[test]
fn hook_output_is_hook_protocol() {
    let r = repo();
    for input in [
        question(&[(12, &head("abc1234"))], "Hold"),
        json!("not an object"),
    ] {
        let out = post(r.path(), &input);
        assert_eq!(out.status.code(), Some(0));
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(!text.contains("\"version\""), "no envelope: {text}");
        assert!(text.trim().is_empty(), "{text}");
    }
    // Garbage on stdin and outside a repo: still exit 0, nothing printed.
    let out = bosun()
        .args(["hook", "post-tool-use"])
        .write_stdin("{")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
}

fn ready(fake: &FakeGithub, n: u64, h: &str) {
    fake.pull(n, "gov", &format!("b{n}"), "");
    fake.pull_as_issue(n, "gov");
    fake.pull_set(
        n,
        "head",
        json!({"sha": h, "ref": format!("b{n}"), "label": format!("acme:b{n}")}),
    );
    fake.status(h, "risk-check", "failure", WAITING);
    fake.status(h, "review-verdict", "success", "approved by session_rev");
    fake.check_run(h, "ci-ok", "completed", Some("success"));
    fake.compare_file("base0000", h, &format!("governance/f{n}.toml"), "@@");
}

fn list(fake: &FakeGithub, repo: &Path) -> Value {
    let out = bosun()
        .current_dir(repo)
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_OWNER_TOKEN", "canary-owner-token-7f3a")
        .env("BOSUN_HTTP_MAX_SLEEP_MS", "0")
        .args(["approve", "--list"])
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    v
}

#[test]
fn held_pr_not_reasked_same_head() {
    let r = repo();
    let fake = FakeGithub::start();
    let (h12, h13) = (head("abc1234"), head("def5678"));
    ready(&fake, 12, &h12);
    ready(&fake, 13, &h13);
    post(r.path(), &question(&[(12, &h12)], "Hold"));
    let v = list(&fake, r.path());
    let prs: Vec<_> = v["data"]["ready"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["pr"].clone())
        .collect();
    assert_eq!(prs, vec![json!(13)], "{v}");
    assert_eq!(v["data"]["held"], json!([{"pr": 12, "head_sha": h12}]));
    assert_eq!(
        v["data"]["approve_command"],
        json!(format!("bosun approve 13@{h13}"))
    );
    let q = v["next"][0]["args"]["questions"][0].to_string();
    assert!(!q.contains("#12"), "{q}");
    // Only the held PR waits: nothing to ask.
    let fake2 = FakeGithub::start();
    ready(&fake2, 12, &h12);
    let v = list(&fake2, r.path());
    assert!(v["next"].as_array().is_none_or(|n| n.is_empty()), "{v}");
}

#[test]
fn held_pr_reasked_new_head() {
    let r = repo();
    let fake = FakeGithub::start();
    let h_old = head("abc1234");
    let h_new = head("fedcba9");
    ready(&fake, 12, &h_new);
    post(r.path(), &question(&[(12, &h_old)], "Hold"));
    let v = list(&fake, r.path());
    assert_eq!(v["data"]["ready"][0]["pr"], json!(12), "{v}");
    assert_eq!(v["data"]["held"], json!([]));
    assert_eq!(v["next"][0]["tool"], json!("AskUserQuestion"));
}
