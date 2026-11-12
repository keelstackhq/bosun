mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

const WAITING: &str = "high risk, owner approval needed: R-gov, R-ci";

fn head(prefix: &str) -> String {
    format!("{prefix:0<40}")
}

fn set_head(fake: &FakeGithub, n: u64, sha: &str) {
    let h = json!({"sha": sha, "ref": format!("b{n}"), "label": format!("acme:b{n}")});
    fake.pull_set(n, "head", h);
    fake.status(sha, "risk-check", "failure", WAITING);
    fake.status(sha, "review-verdict", "success", "approved");
    fake.check_run(sha, "ci-ok", "completed", Some("success"));
    fake.compare_file("base0000", sha, &format!("governance/f{n}.toml"), "@@");
}

/// PR `n` at `sha`, ready for an owner.
fn ready(fake: &FakeGithub, n: u64, title: &str, sha: &str) {
    fake.pull(n, title, &format!("b{n}"), "");
    fake.pull_as_issue(n, title);
    set_head(fake, n, sha);
}

fn setup() -> FakeGithub {
    let fake = FakeGithub::start();
    ready(&fake, 15, "second change", &head("def5678"));
    ready(&fake, 12, "first change", &head("abc1234"));
    ready(&fake, 20, "fix main", &head("fff0000"));
    fake.pull_set(20, "labels", json!([{"name": "bosun:kind/mainfix"}]));
    fake.issue(30, "which db?", &["bosun:state/needs-human"], "body");
    fake.issue(40, "main is red", &["bosun:main-red"], "");
    fake
}

fn digest(fake: &FakeGithub, extra: &[&str], input: &str) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_OWNER_TOKEN", "canary-owner-token-7f3a")
        .env("BOSUN_HTTP_MAX_SLEEP_MS", "0")
        .env("BOSUN_SESSION", "session_orch")
        .env("BOSUN_NOW", "2026-11-12T06:00:00Z")
        .arg("digest")
        .args(extra)
        .write_stdin(input.to_string())
        .output()
        .unwrap();
    let all = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(!all.contains("canary-owner-token"), "token leaked");
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(
            !v["error"]["fix"].as_str().unwrap().trim().is_empty(),
            "{v}"
        );
    }
    (out.status.code().unwrap(), v)
}

fn writes(fake: &FakeGithub) -> usize {
    let r = fake.requests();
    r.iter().filter(|r| r.method != "GET").count()
}

#[test]
fn the_digest_lists_owner_waiting_prs_and_open_asks_and_skips_mainfix() {
    let fake = setup();
    let (code, v) = digest(&fake, &[], "");
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    let id = d["message_id"].as_str().unwrap();
    let want = format!(
        "Daily digest acme/widget {id}\nApprovals (2):\n- #12@abc1234 R-gov, R-ci: `first change`\n- #15@def5678 R-gov, R-ci: `second change`\nOpen asks (1):\n- #30 `which db?`\nBudget asks (0):\nReply with (drop the PRs you hold):\n{id} approve 12 15"
    );
    assert_eq!(d["message"], json!(want));
    let pairs = format!("12@{} 15@{}", head("abc1234"), head("def5678"));
    assert_eq!(
        d["header"],
        json!(format!("/bosun digest acme/widget {id} {pairs}"))
    );
    assert_eq!(
        d["bypass"],
        json!([{"pr": 20, "why": "mainfix: approve it now, not in the digest"}])
    );
    assert_eq!(
        d["asks"],
        json!([{"issue": 30, "answer": "bosun answer 30 --chat"}])
    );
    assert_eq!(d["skipped"], json!([]));
    assert_eq!(writes(&fake), 0, "the digest writes nothing");
    // On a green main a mainfix PR is in the digest like any other.
    let fake = setup();
    fake.close_issue(40, "2026-11-12T00:00:00Z");
    let (_, v) = digest(&fake, &[], "");
    assert_eq!(v["data"]["bypass"], json!([]));
    assert_eq!(v["data"]["items"].as_array().unwrap().len(), 3);
}

#[test]
fn the_answer_returns_approve_calls_and_drops_a_moved_head() {
    let fake = setup();
    let (_, v) = digest(&fake, &[], "");
    let header = v["data"]["header"].as_str().unwrap().to_string();
    let id = v["data"]["message_id"].as_str().unwrap().to_string();
    // The form's reply line, pasted as is, parses.
    let line = v["data"]["message"]
        .as_str()
        .unwrap()
        .lines()
        .last()
        .unwrap()
        .to_string();
    let (code, v) = digest(&fake, &["--answer"], &format!("{header}\n{line}\n"));
    assert_eq!((code, v["data"]["unknown"].clone()), (0, json!([])), "{v}");
    assert_eq!(v["data"]["approve"].as_array().unwrap().len(), 2);
    let moved = head("def9999");
    set_head(&fake, 15, &moved);
    let input = format!("{header}\n{id} Approve 12, 15, 20\nraise 7 +20\nthanks\n");
    let (code, v) = digest(&fake, &["--answer"], &input);
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    let cmd = format!("bosun approve 12@{} --sign-key <pem>", head("abc1234"));
    assert_eq!(
        v["next"],
        json!([{"tool": "Bash", "args": {"command": cmd}}])
    );
    assert_eq!(
        d["approve"],
        json!([{"pr": 12, "head_sha": head("abc1234")}])
    );
    let dropped: Vec<u64> = d["dropped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["pr"].as_u64().unwrap())
        .collect();
    assert_eq!(dropped, vec![15, 20]);
    assert_eq!(d["unknown"], json!(["thanks"]));
    assert_eq!(d["refused"][0]["raise"], json!({"epic": 7, "usd": 20}));
    assert_eq!(writes(&fake), 0, "the answer writes nothing");
    // A quoted id of another digest is a conflict.
    let other = format!("{header}\n0000beef approve 12\n");
    let (code, v) = digest(&fake, &["--answer"], &other);
    assert_eq!(code, 3, "{v}");
    // Every quoted id must match, and a reply without one is refused.
    let mixed = format!("{header}\n{id} approve 12\n0000beef approve 15\n");
    let (code, v) = digest(&fake, &["--answer"], &mixed);
    assert_eq!(code, 3, "{v}");
    let (code, v) = digest(&fake, &["--answer"], &format!("{header}\napprove 12\n"));
    assert_eq!(code, 2, "{v}");
}

#[test]
fn an_answer_without_a_valid_header_is_a_usage_error() {
    let fake = setup();
    let (_, v) = digest(&fake, &[], "");
    let header = v["data"]["header"].as_str().unwrap();
    let other_repo = header.replace("acme/widget", "acme/other");
    let other_repo = format!("{other_repo}\napprove 12\n");
    let fine = format!("{header}\napprove 12\n");
    for input in [
        "approve 12\n",
        "/bosun digest acme/widget 00000000 12@abc\napprove 12\n",
        other_repo.as_str(),
    ] {
        let (code, v) = digest(&fake, &["--answer"], input);
        assert_eq!(code, 2, "{v}");
    }
    // A halted repo answers nothing.
    fake.issue(50, "halt", &["bosun:halt"], "");
    let (code, _) = digest(&fake, &["--answer"], &fine);
    assert_eq!(code, 4);
    // A halted repo signs nothing either.
    let (code, v) = digest(&fake, &["--answer", "--sign-key", "k.pem"], &fine);
    assert_eq!(code, 4, "{v}");
}
