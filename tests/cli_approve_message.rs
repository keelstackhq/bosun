mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

const WAITING: &str = "high risk, owner approval needed: R-gov, R-ci";

/// PR `n` at head `head`, ready for an owner: risk-check waits, verdict and ci-ok are green.
fn ready(fake: &FakeGithub, n: u64, title: &str, head: &str) {
    fake.pull(n, title, &format!("b{n}"), "");
    fake.pull_as_issue(n, title);
    set_head(fake, n, head);
    fake.status(head, "risk-check", "failure", WAITING);
    fake.status(head, "review-verdict", "success", "approved");
    fake.check_run(head, "ci-ok", "completed", Some("success"));
    fake.compare_file("base0000", head, &format!("governance/f{n}.toml"), "@@");
}

fn set_head(fake: &FakeGithub, n: u64, head: &str) {
    fake.pull_set(
        n,
        "head",
        json!({"sha": head, "ref": format!("b{n}"), "label": format!("acme:b{n}")}),
    );
}

fn list(fake: &FakeGithub) -> Value {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_OWNER_TOKEN", "canary-owner-token-7f3a")
        .env("BOSUN_HTTP_MAX_SLEEP_MS", "0")
        .args(["approve", "--list"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    one_object(&out.stdout)["data"].clone()
}

fn head(prefix: &str) -> String {
    format!("{prefix:0<40}")
}

#[test]
fn the_message_lists_the_ready_prs_and_the_reply() {
    let fake = FakeGithub::start();
    ready(&fake, 15, "second change", &head("def5678"));
    ready(
        &fake,
        12,
        &format!("M1/T3: {}", "x".repeat(60)),
        &head("abc1234"),
    );
    let data = list(&fake);
    let id = data["message_id"].as_str().unwrap();
    assert_eq!(id.len(), 4);
    assert!(id.bytes().all(|b| b.is_ascii_hexdigit()), "{id}");
    let cut = format!("M1/T3: {}…", "x".repeat(42));
    assert_eq!(cut.chars().count(), 50);
    let want = format!(
        "2 PRs wait for your approval (message {id}):\n- #12@abc1234 {cut}\n- #15@def5678 second change\nReply: approve 12 15"
    );
    assert_eq!(data["message"], json!(want));
    // Same heads, same id.
    assert_eq!(list(&fake)["message_id"], json!(id));
    // A moved head, a new id.
    let moved = head("abc1235");
    fake.status(&moved, "risk-check", "failure", WAITING);
    fake.status(&moved, "review-verdict", "success", "approved");
    fake.check_run(&moved, "ci-ok", "completed", Some("success"));
    fake.compare_file("base0000", &moved, "governance/f12.toml", "@@");
    set_head(&fake, 12, &moved);
    assert_ne!(list(&fake)["message_id"], json!(id));
}

#[test]
fn one_pr_waits_and_nothing_ready_is_null() {
    let fake = FakeGithub::start();
    let data = list(&fake);
    assert!(data["message"].is_null());
    assert!(data["message_id"].is_null());
    ready(&fake, 7, "only\tone", &head("1234567"));
    let data = list(&fake);
    let id = data["message_id"].as_str().unwrap();
    assert_eq!(
        data["message"],
        json!(format!(
            "1 PR waits for your approval (message {id}):\n- #7@1234567 only one\nReply: approve 7"
        ))
    );
}

#[test]
fn more_than_ten_are_counted_not_listed() {
    let fake = FakeGithub::start();
    for n in 1..=12u64 {
        ready(&fake, n, &format!("change {n}"), &head(&format!("{n:07x}")));
    }
    let msg = list(&fake)["message"].as_str().unwrap().to_string();
    let lines: Vec<&str> = msg.lines().collect();
    assert!(lines[0].starts_with("12 PRs wait for your approval (message "));
    assert_eq!(lines.len(), 13);
    assert_eq!(lines[11], "- … and 2 more");
    assert_eq!(lines[12], "Reply: approve 1 2 3 4 5 6 7 8 9 10");
}
