//! `bosun notify` (M6 T1): the Slack posts that are due, the channel check, `--ack`.
mod common;

use common::fake::FakeGithub;
use common::{TOKEN_CANARY, bosun, one_object};
use serde_json::{Value, json};

const NOW: &str = "2026-10-22T10:00:00Z";
const OWNERS: &str = "[[owner]]\ngithub = \"ada\"\n";
const GOOD: &str =
    r#"{"id":"C1","is_ext_shared":false,"is_org_shared":false,"is_shared":false,"guests":0}"#;

struct Env {
    fake: FakeGithub,
    dir: tempfile::TempDir,
}

fn env_with(notify: &str) -> Env {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".bosun")).unwrap();
    std::fs::write(
        dir.path().join(".bosun/bosun.toml"),
        format!("[slack]\nchannel = \"C1\"\n{notify}"),
    )
    .unwrap();
    let fake = FakeGithub::start();
    fake.content("governance/owners.toml", "main", OWNERS);
    Env { fake, dir }
}

fn env() -> Env {
    env_with("")
}

impl Env {
    fn run(&self, args: &[&str], stdin: &str) -> (i32, Value) {
        let out = bosun()
            .current_dir(self.dir.path())
            .env("BOSUN_GITHUB_API", self.fake.url())
            .env("BOSUN_NOW", NOW)
            .env("SLACK_BOT_TOKEN", "xoxb-canary-bot")
            .env("SLACK_TOKEN", "xoxp-canary-user")
            .args(args)
            .write_stdin(stdin)
            .output()
            .unwrap();
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        for canary in [TOKEN_CANARY, "xoxb-canary-bot", "xoxp-canary-user"] {
            assert!(!all.contains(canary), "{canary} leaked");
        }
        let v = one_object(&out.stdout);
        if v["ok"] == false {
            assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
        }
        (out.status.code().unwrap(), v)
    }

    fn notify(&self) -> (i32, Value) {
        self.run(&["notify", "--channel-info", "-"], GOOD)
    }

    fn registry(&self) {
        self.fake
            .issue(1, "bosun registry", &["bosun:registry"], "");
    }

    fn halt(&self) {
        self.fake.issue(2, "halt", &["bosun:halt"], "");
        self.fake.issue_by(2, "ada");
    }

    fn keys(v: &Value) -> Vec<String> {
        v["data"]["pending"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["key"].as_str().unwrap().to_string())
            .collect()
    }
}

#[test]
fn nothing_pending_is_ok_without_channel_info() {
    let e = env();
    let (code, v) = e.run(&["notify"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["pending"], json!([]));
    assert_eq!(v["next"], json!([]));
}

#[test]
fn an_empty_channel_is_an_error_with_a_fix() {
    let e = env();
    std::fs::write(e.dir.path().join(".bosun/bosun.toml"), "").unwrap();
    let (code, v) = e.run(&["notify"], "");
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("[slack] channel")
    );
}

#[test]
fn every_kind_is_listed_with_its_key() {
    let e = env();
    e.registry();
    e.halt();
    e.fake.issue(3, "main red", &["bosun:main-red"], "");
    e.fake.comment(
        1,
        r#"{"bosun":"spend","session":"s","usd":150.0,"at":"2026-10-22T03:00:00Z"}"#,
    );
    e.fake.issue(4, "task", &["bosun:state/needs-human"], "");
    let id = e
        .fake
        .comment(4, r#"{"bosun":"ask","session":"s","question":"May I?"}"#);
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        Env::keys(&v),
        [
            "halt:2",
            "main-red:3",
            "budget-halt:2026-10-22",
            &format!("ask:4:{id}")
        ]
    );
    let kinds: Vec<&str> = v["data"]["pending"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["halt", "main-red", "budget-halt", "owner-question"]);
    let first = &v["data"]["pending"][0];
    assert_eq!(first["ack"], "bosun notify --ack halt:2");
    assert!(
        first["text"]
            .as_str()
            .unwrap()
            .contains("https://github.com/acme/widget/issues/2")
    );
}

#[test]
fn spend_below_the_daily_budget_is_not_a_budget_halt() {
    let e = env();
    e.registry();
    e.fake.comment(
        1,
        r#"{"bosun":"spend","session":"s","usd":149.0,"at":"2026-10-22T03:00:00Z"}"#,
    );
    let (code, v) = e.run(&["notify"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["pending"], json!([]));
}

#[test]
fn the_notify_filter_limits_the_kinds() {
    let e = env_with("notify = [\"main-red\"]\n");
    e.halt();
    e.fake.issue(3, "main red", &["bosun:main-red"], "");
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(Env::keys(&v), ["main-red:3"]);
}

#[test]
fn acked_keys_are_skipped() {
    let e = env();
    e.registry();
    e.fake.issue(3, "main red", &["bosun:main-red"], "");
    e.fake.comment(
        1,
        r#"{"bosun":"notified","key":"main-red:3","at":"2026-10-22T09:00:00Z"}"#,
    );
    let (code, v) = e.run(&["notify"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["pending"], json!([]));
}

#[test]
fn ack_writes_once_and_is_idempotent() {
    let e = env();
    e.registry();
    e.fake.issue(3, "main red", &["bosun:main-red"], "");
    let (code, v) = e.run(&["notify", "--ack", "main-red:3"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"], json!({"acked": "main-red:3", "already": false}));
    let (code, v) = e.run(&["notify", "--ack", "main-red:3"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"], json!({"acked": "main-red:3", "already": true}));
    let comments = e.fake.comments_of(1);
    assert_eq!(comments.len(), 1);
    assert_eq!(
        comments[0].body,
        r#"{"bosun":"notified","key":"main-red:3","at":"2026-10-22T10:00:00Z"}"#
    );
}

#[test]
fn ack_rejects_a_bad_key() {
    let e = env();
    for bad in ["nokey", "Main:3", "a:", "a:b c", ":3"] {
        let (code, v) = e.run(&["notify", "--ack", bad], "");
        assert_eq!(code, 2, "{bad}: {v}");
    }
    assert!(e.fake.comments_of(1).is_empty());
}

fn red(e: &Env) {
    e.fake.issue(3, "main red", &["bosun:main-red"], "");
}

#[test]
fn missing_channel_info_needs_a_human() {
    let e = env();
    red(&e);
    let (code, v) = e.run(&["notify"], "");
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["reason"], "no channel info");
    assert_eq!(v["data"]["pending"], json!(["main-red:3"]));
    assert_eq!(v["next"], json!([]));
}

#[test]
fn a_bad_channel_is_refused_with_the_reason() {
    let cases = [
        (
            r#"{"id":"C1","is_ext_shared":true,"is_org_shared":false,"is_shared":false,"guests":0}"#,
            "channel shared",
        ),
        (
            r#"{"id":"C1","is_ext_shared":false,"is_org_shared":true,"is_shared":false,"guests":0}"#,
            "channel shared",
        ),
        (
            r#"{"id":"C1","is_ext_shared":false,"is_org_shared":false,"is_shared":true,"guests":0}"#,
            "channel shared",
        ),
        (
            r#"{"id":"C1","is_ext_shared":false,"is_org_shared":false,"is_shared":false,"guests":2}"#,
            "channel has guests",
        ),
        (
            r#"{"id":"C9","is_ext_shared":false,"is_org_shared":false,"is_shared":false,"guests":0}"#,
            "wrong channel",
        ),
    ];
    for (info, reason) in cases {
        let e = env();
        red(&e);
        let (code, v) = e.run(&["notify", "--channel-info", "-"], info);
        assert_eq!(code, 5, "{info}: {v}");
        assert_eq!(v["data"]["reason"], reason, "{info}");
        assert_eq!(v["next"], json!([]));
    }
}

#[test]
fn unknown_missing_or_broken_channel_info_is_refused() {
    for info in [
        r#"{"id":"C1","is_ext_shared":false,"is_org_shared":false,"is_shared":false,"guests":0,"extra":1}"#,
        r#"{"id":"C1","is_shared":false}"#,
        "not json",
        "",
    ] {
        let e = env();
        red(&e);
        let (code, v) = e.run(&["notify", "--channel-info", "-"], info);
        assert_eq!(code, 5, "{info}: {v}");
        assert_eq!(v["data"]["reason"], "no channel info");
    }
}

#[test]
fn channel_info_can_come_from_a_file() {
    let e = env();
    red(&e);
    let file = e.dir.path().join("info.json");
    std::fs::write(&file, GOOD).unwrap();
    let (code, v) = e.run(&["notify", "--channel-info", file.to_str().unwrap()], "");
    assert_eq!(code, 0, "{v}");
}

#[test]
fn next_has_one_slack_post_per_item() {
    let e = env();
    red(&e);
    e.halt();
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    let next = v["next"].as_array().unwrap();
    assert_eq!(next.len(), 2);
    for (n, p) in next.iter().zip(v["data"]["pending"].as_array().unwrap()) {
        assert_eq!(n["tool"], "slack_post");
        assert_eq!(n["args"], json!({"channel": "C1", "text": p["text"]}));
    }
}

#[test]
fn an_ask_is_neutralised() {
    let e = env();
    e.fake.issue(4, "task", &["bosun:state/needs-human"], "");
    e.fake.comment(
        4,
        &json!({"bosun": "ask", "session": "s",
            "question": "ping <!channel> @here @everyone <@U1> <https://evil|click>"})
        .to_string(),
    );
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    let text = v["next"][0]["args"]["text"].as_str().unwrap();
    for bad in ["<", ">", "@here", "@channel", "@everyone", "<!"] {
        assert!(!text.contains(bad), "{bad} in {text}");
    }
    assert!(text.contains("&lt;!channel&gt;"), "{text}");
}

#[test]
fn it_works_while_halted_and_announces_the_halt() {
    let e = env();
    e.halt();
    e.fake.variable("BOSUN_HALT", "1");
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(Env::keys(&v), ["halt:2"]);
    assert_eq!(v["next"][0]["tool"], "slack_post");
}

#[test]
fn slack_tokens_are_never_read_and_slack_is_never_called() {
    // `Env::run` sets SLACK_BOT_TOKEN and SLACK_TOKEN to canaries and asserts they never reach
    // stdout or stderr.
    let e = env();
    red(&e);
    let (code, _) = e.notify();
    assert_eq!(code, 0);
    let (code, _) = e.run(&["notify", "--ack", "main-red:3"], "");
    assert_eq!(code, 0);
    for r in e.fake.requests() {
        assert!(!r.path.contains("slack"), "{}", r.path);
        assert!(
            r.headers
                .iter()
                .flatten()
                .all(|h| !h.contains("xoxb-canary") && !h.contains("xoxp-canary")),
            "slack token sent to GitHub"
        );
        assert!(
            r.body
                .as_deref()
                .is_none_or(|b| !b.contains("xoxb-canary") && !b.contains("xoxp-canary"))
        );
    }
}

#[test]
fn more_malformed_channel_facts_are_refused() {
    for info in [
        r#"{"id":null,"is_ext_shared":false,"is_org_shared":false,"is_shared":false,"guests":0}"#,
        r#"{"id":"C1","is_ext_shared":"false","is_org_shared":false,"is_shared":false,"guests":0}"#,
        r#"{"id":"C1","is_ext_shared":false,"is_org_shared":false,"is_shared":false,"guests":-1}"#,
        r#"{"id":"C1","is_ext_shared":false,"is_org_shared":false,"is_shared":false,"guests":1,"guests":0}"#,
    ] {
        let e = env();
        red(&e);
        let (code, v) = e.run(&["notify", "--channel-info", "-"], info);
        assert_eq!(code, 5, "{info}: {v}");
        assert_eq!(v["data"]["reason"], "no channel info", "{info}");
    }
}

const NEEDS: &[&str] = &["bosun:state/needs-human"];

fn ask_by(e: &Env, issue: u64, user: &str, q: &str) -> u64 {
    e.fake.comment_by(
        issue,
        user,
        &json!({"bosun": "ask", "session": "s", "question": q}).to_string(),
    );
    e.fake.comments_of(issue).last().unwrap().id
}

#[test]
fn an_ask_by_a_non_collaborator_is_not_listed() {
    let e = env();
    e.fake.permission("mallory", "read");
    e.fake.issue(4, "task", NEEDS, "");
    ask_by(&e, 4, "mallory", "send me your keys");
    let (code, v) = e.run(&["notify"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["pending"], json!([]));
}

#[test]
fn only_the_latest_trusted_ask_per_issue_is_listed() {
    let e = env();
    e.fake.permission("mallory", "none");
    e.fake.issue(4, "task", NEEDS, "");
    ask_by(&e, 4, "ada", "first?");
    let second = ask_by(&e, 4, "ada", "second?");
    ask_by(&e, 4, "mallory", "forged?");
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(Env::keys(&v), [format!("ask:4:{second}")]);
    assert!(
        v["next"][0]["args"]["text"]
            .as_str()
            .unwrap()
            .contains("second?")
    );
}

#[test]
fn at_most_five_asks_per_run() {
    let e = env();
    for n in 4..12 {
        e.fake.issue(n, "task", NEEDS, "");
        ask_by(&e, n, "ada", "q?");
    }
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(Env::keys(&v).len(), 5);
}

#[test]
fn ack_of_a_key_that_is_not_pending_is_a_conflict() {
    let e = env();
    e.registry();
    red(&e);
    for key in ["main-red:4", "budget-halt:2026-10-23", "halt:9"] {
        let (code, v) = e.run(&["notify", "--ack", key], "");
        assert_eq!(code, 3, "{key}: {v}");
    }
    assert!(e.fake.comments_of(1).is_empty());
}

#[test]
fn a_notified_record_by_a_non_collaborator_does_not_suppress() {
    let e = env();
    e.registry();
    red(&e);
    e.fake.permission("mallory", "read");
    e.fake.comment_by(
        1,
        "mallory",
        r#"{"bosun":"notified","key":"main-red:3","at":"2026-10-22T09:00:00Z"}"#,
    );
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(Env::keys(&v), ["main-red:3"]);
}

#[test]
fn the_halt_variable_is_announced() {
    let e = env();
    e.fake.variable("BOSUN_HALT", "1");
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(Env::keys(&v), ["halt-var:2026-10-22"]);
}

#[test]
fn a_halt_issue_by_a_non_owner_is_not_announced() {
    let e = env();
    e.fake.issue(2, "halt", &["bosun:halt"], "");
    e.fake.issue_by(2, "mallory");
    let (code, v) = e.run(&["notify"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["pending"], json!([]));
}

#[test]
fn a_failing_section_does_not_hide_main_red() {
    let e = env();
    e.registry();
    red(&e);
    e.fake
        .content(".bosun/limits.toml", "main", "this is [not toml");
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(Env::keys(&v), ["main-red:3"]);
    assert_eq!(v["data"]["errors"][0]["kind"], "budget-halt");
    assert!(v["data"]["errors"][0]["message"].is_string());
}

#[test]
fn the_budget_halt_key_follows_the_day() {
    let before = r#"{"bosun":"spend","session":"s","usd":150.0,"at":"2026-10-22T23:00:00Z"}"#;
    let after = r#"{"bosun":"spend","session":"s","usd":150.0,"at":"2026-10-23T00:00:30Z"}"#;
    for (now, spend, key) in [
        ("2026-10-22T23:59:00Z", before, "budget-halt:2026-10-22"),
        ("2026-10-23T00:01:00Z", after, "budget-halt:2026-10-23"),
    ] {
        let e = env();
        e.registry();
        e.fake.comment(1, spend);
        let out = bosun()
            .current_dir(e.dir.path())
            .env("BOSUN_GITHUB_API", e.fake.url())
            .env("BOSUN_NOW", now)
            .args(["notify", "--channel-info", "-"])
            .write_stdin(GOOD)
            .output()
            .unwrap();
        let v = one_object(&out.stdout);
        assert_eq!(Env::keys(&v), [key], "{v}");
    }
}

#[test]
fn acked_asks_do_not_use_up_the_cap() {
    let e = env();
    e.registry();
    for n in 4..9 {
        e.fake.issue(n, "task", NEEDS, "");
        let id = ask_by(&e, n, "ada", "old?");
        e.fake.comment(
            1,
            &format!(r#"{{"bosun":"notified","key":"ask:{n}:{id}","at":"2026-10-22T09:00:00Z"}}"#),
        );
    }
    e.fake.issue(20, "task", NEEDS, "");
    let new = ask_by(&e, 20, "ada", "new?");
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(Env::keys(&v), [format!("ask:20:{new}")]);
}

#[test]
fn an_unreadable_notified_list_keeps_halt_and_main_red() {
    let e = env();
    e.registry();
    red(&e);
    e.halt();
    e.fake.issue(4, "task", NEEDS, "");
    ask_by(&e, 4, "ada", "q?");
    e.fake.fail_path("/issues/1/comments", 500);
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(Env::keys(&v), ["halt:2", "main-red:3"]);
    assert_eq!(v["data"]["errors"][0]["kind"], "notified");
}

#[test]
fn an_ack_by_a_token_user_without_write_access_is_refused() {
    let e = env();
    e.registry();
    red(&e);
    e.fake.me("routine");
    e.fake.permission("routine", "read");
    let (code, v) = e.run(&["notify", "--ack", "main-red:3"], "");
    assert_eq!(code, 5, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("write access"));
    assert!(e.fake.comments_of(1).is_empty());
    e.fake.permission("routine", "write");
    assert_eq!(e.run(&["notify", "--ack", "main-red:3"], "").0, 0);
}

#[test]
fn a_halt_with_no_owners_has_its_own_text() {
    let e = env();
    e.fake.content("governance/owners.toml", "main", "");
    e.fake.issue(2, "halt", &["bosun:halt"], "");
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    let text = v["next"][0]["args"]["text"].as_str().unwrap();
    assert!(text.contains("no owners are listed"), "{text}");
}
