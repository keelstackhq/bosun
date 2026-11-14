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
    fake.content("governance/owners.toml", "refs/heads/main", OWNERS);
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
    std::fs::write(
        e.dir.path().join(".bosun/bosun.toml"),
        "[slack]\nchannel = \"\"\n",
    )
    .unwrap();
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
    // The fake writes comments as "octocat".
    e.fake.permission("octocat", "none");
    let (code, v) = e.run(&["notify", "--ack", "main-red:3"], "");
    assert_eq!(code, 5, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("write access"));
    // The ignored record leaves the key pending.
    let (code, v) = e.run(&["notify"], "");
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["pending"], json!(["main-red:3"]));
}

#[test]
fn a_halt_with_no_owners_has_its_own_text() {
    let e = env();
    e.fake
        .content("governance/owners.toml", "refs/heads/main", "");
    e.fake.issue(2, "halt", &["bosun:halt"], "");
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    let text = v["next"][0]["args"]["text"].as_str().unwrap();
    assert!(text.contains("no owners are listed"), "{text}");
}

#[test]
fn without_a_slack_section_notify_is_skipped_not_an_error() {
    let e = env();
    e.registry();
    e.halt();
    std::fs::write(e.dir.path().join(".bosun/bosun.toml"), "").unwrap();
    let (code, v) = e.run(&["notify"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["skipped"], true, "{v}");
    assert!(
        v["data"]["reason"].as_str().unwrap().contains("[slack]"),
        "{v}"
    );
    assert_eq!(v["data"]["pending"], json!([]));
    assert_eq!(v["next"], json!([]));
}

const TS: &str = "1700000000.000200";
const TS2: &str = "1700000000.000300";

/// A needs-human task whose newest lease came from thread `ts`, with an ask; returns the key.
fn threaded_ask(e: &Env, n: u64, ts: &str) -> String {
    e.fake.issue(n, "task", NEEDS, "");
    e.fake.comment(
        n,
        &json!({"bosun": "lease", "session": "s", "origin": "slack",
            "expires": "2026-10-22T09:00:00Z", "touch": [], "thread_ts": ts})
        .to_string(),
    );
    let id = ask_by(e, n, "ada", "which way?");
    format!("ask:{n}:{id}")
}

/// A join record for `session_test` with a Claude Tag seat, by `user`, expiring at `expires`.
fn seat_by(e: &Env, user: &str, expires: &str, seat: Value) {
    e.registry();
    e.fake.comment_by(
        1,
        user,
        &json!({"bosun": "join", "session": "session_test", "origin": "slack", "depth": 1,
            "expires": expires, "slack": seat})
        .to_string(),
    );
}

/// This session (`session_test`) joined with a Claude Tag seat.
fn seat(e: &Env, seat: Value) {
    seat_by(e, "ada", "2026-10-22T10:30:00Z", seat);
}

#[test]
fn an_ask_on_a_thread_task_is_posted_into_its_thread() {
    let e = env();
    let key = threaded_ask(&e, 4, TS);
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(Env::keys(&v), [key]);
    assert_eq!(v["data"]["pending"][0]["thread_ts"], TS);
    assert_eq!(v["next"][0]["tool"], "slack_post");
    assert_eq!(v["next"][0]["args"]["thread_ts"], TS);
    assert_eq!(v["data"]["seat"], Value::Null);
}

#[test]
fn a_channel_seat_routes_thread_posts_through_send_message() {
    let e = env();
    threaded_ask(&e, 4, TS);
    e.fake.issue(5, "plain", NEEDS, "");
    ask_by(&e, 5, "ada", "plain?");
    seat(&e, json!({"role": "channel", "channel": "C1"}));
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["seat"]["role"], "channel");
    let next = v["next"].as_array().unwrap();
    assert_eq!(next.len(), 2);
    assert_eq!(next[0]["tool"], "send_message");
    assert_eq!(next[0]["args"]["to"], "thread");
    assert_eq!(next[0]["args"]["thread_ts"], TS);
    assert!(
        next[0]["args"]["message"]
            .as_str()
            .unwrap()
            .contains("which way?")
    );
    assert_eq!(next[1]["tool"], "slack_post");
    assert!(next[1]["args"].get("thread_ts").is_none());
}

#[test]
fn a_thread_seat_lists_its_thread_and_channel_posts_only() {
    let e = env();
    e.halt();
    threaded_ask(&e, 4, TS);
    threaded_ask(&e, 5, TS2);
    e.fake.issue(6, "plain", NEEDS, "");
    ask_by(&e, 6, "ada", "plain?");
    seat(
        &e,
        json!({"role": "thread", "channel": "C1", "thread_ts": TS}),
    );
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["seat"]["thread_ts"], TS);
    let tools: Vec<(&str, &Value)> = v["next"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| (n["tool"].as_str().unwrap(), &n["args"]))
        .collect();
    assert_eq!(tools.len(), 3, "{v}");
    assert_eq!(tools[0].0, "slack_post"); // halt
    let reply = tools.iter().find(|t| t.0 == "thread_reply").unwrap();
    assert_eq!(reply.1["thread_ts"], TS);
    assert!(reply.1["text"].as_str().unwrap().contains("which way?"));
    assert_eq!(tools.iter().filter(|t| t.0 == "slack_post").count(), 2);
    assert!(!v.to_string().contains("#5:"), "{v}");
}

#[test]
fn an_explicit_thread_beats_the_thread_seat() {
    let e = env();
    threaded_ask(&e, 4, TS);
    e.fake.comment(
        4,
        &json!({"bosun": "result", "session": "s", "outcome": "merged"}).to_string(),
    );
    seat(
        &e,
        json!({"role": "thread", "channel": "C1", "thread_ts": TS2}),
    );
    let (code, v) = e.run(&["notify", "--channel-info", "-", "--thread", TS], GOOD);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["seat"]["role"], "thread");
    assert_eq!(v["next"][0]["tool"], "slack_post");
}

#[test]
fn a_missing_registry_leaves_the_posts_alone() {
    let e = env();
    threaded_ask(&e, 4, TS);
    // No registry issue: no seat, no error, plain slack_post.
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["seat"], Value::Null);
    assert_eq!(v["next"][0]["tool"], "slack_post");
}

#[test]
fn a_seat_written_by_a_non_writer_is_ignored() {
    let e = env();
    e.fake.permission("mallory", "none");
    threaded_ask(&e, 4, TS);
    seat_by(
        &e,
        "mallory",
        "2026-10-22T10:30:00Z",
        json!({"role": "channel", "channel": "C1"}),
    );
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["seat"], Value::Null);
    assert_eq!(v["next"][0]["tool"], "slack_post");
}

#[test]
fn an_expired_seat_is_ignored() {
    let e = env();
    threaded_ask(&e, 4, TS);
    seat_by(
        &e,
        "ada",
        "2026-10-22T09:00:00Z",
        json!({"role": "channel", "channel": "C1"}),
    );
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["seat"], Value::Null);
    assert_eq!(v["next"][0]["tool"], "slack_post");
}

#[test]
fn a_thread_seats_ask_is_not_cut_by_other_threads_asks() {
    let e = env();
    for n in 4..=8 {
        threaded_ask(&e, n, TS2);
    }
    let own = threaded_ask(&e, 9, TS);
    seat(
        &e,
        json!({"role": "thread", "channel": "C1", "thread_ts": TS}),
    );
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(Env::keys(&v), [own]);
    assert_eq!(v["next"][0]["tool"], "thread_reply");
}

#[test]
fn an_unreadable_registry_is_reported_as_a_seat_error() {
    let e = env();
    e.registry();
    e.halt();
    e.fake.fail_path("/issues/1/comments", 500);
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    let errors = v["data"]["errors"].as_array().unwrap();
    assert!(errors.iter().any(|x| x["kind"] == "seat"), "{v}");
    assert_eq!(Env::keys(&v), ["halt:2"]);
    assert_eq!(v["next"][0]["tool"], "slack_post");
    assert_eq!(v["data"]["seat"], Value::Null);
}

#[test]
fn an_explicit_thread_posts_plainly_even_with_a_thread_seat() {
    let e = env();
    threaded_ask(&e, 4, TS);
    e.fake.comment(
        4,
        &json!({"bosun": "result", "session": "s", "outcome": "merged", "pr": 7}).to_string(),
    );
    seat(
        &e,
        json!({"role": "thread", "channel": "C1", "thread_ts": TS2}),
    );
    let (code, v) = e.run(&["notify", "--channel-info", "-", "--thread", TS], GOOD);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["next"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["next"][0]["tool"], "slack_post");
    assert_eq!(v["next"][0]["args"]["thread_ts"], TS);
}

#[test]
fn a_lease_by_a_non_writer_gives_an_ask_no_thread() {
    let e = env();
    e.fake.permission("mallory", "none");
    e.fake.issue(4, "task", NEEDS, "");
    e.fake.comment_by(
        4,
        "mallory",
        &json!({"bosun": "lease", "session": "s", "origin": "slack",
            "expires": "2026-10-22T09:00:00Z", "touch": [], "thread_ts": TS})
        .to_string(),
    );
    ask_by(&e, 4, "ada", "q?");
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["pending"][0]["thread_ts"], Value::Null);
    assert!(v["next"][0]["args"].get("thread_ts").is_none());
}

#[test]
fn a_routed_key_can_be_acked() {
    let e = env();
    let key = threaded_ask(&e, 4, TS);
    seat(&e, json!({"role": "channel", "channel": "C1"}));
    let (_, v) = e.notify();
    assert_eq!(v["next"][0]["tool"], "send_message");
    let (code, v) = e.run(&["notify", "--ack", &key], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["already"], false);
    let (_, v) = e.run(&["notify"], "");
    assert_eq!(v["data"]["pending"], json!([]));
}

#[test]
fn an_over_cap_own_ask_can_be_acked() {
    let e = env();
    for n in 4..=8 {
        threaded_ask(&e, n, TS2);
    }
    let own = threaded_ask(&e, 9, TS);
    seat(
        &e,
        json!({"role": "thread", "channel": "C1", "thread_ts": TS}),
    );
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(Env::keys(&v), std::slice::from_ref(&own));
    let (code, v) = e.run(&["notify", "--ack", &own], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["already"], false);
    let (_, v) = e.notify();
    assert_eq!(Env::keys(&v), Vec::<String>::new());
}

const WAITING: &str = "high risk, owner approval needed: R-gov, R-ci";

fn head40(prefix: &str) -> String {
    format!("{prefix:0<40}")
}

/// PR `n` at the head `prefix`, ready for an owner.
fn waiting_pr(e: &Env, n: u64, title: &str, prefix: &str) {
    let sha = head40(prefix);
    e.fake.pull(n, title, &format!("b{n}"), "");
    e.fake.pull_as_issue(n, title);
    let h = json!({"sha": sha, "ref": format!("b{n}"), "label": format!("acme:b{n}")});
    e.fake.pull_set(n, "head", h);
    e.fake.status(&sha, "risk-check", "failure", WAITING);
    e.fake.status(&sha, "review-verdict", "success", "approved");
    e.fake
        .check_run(&sha, "ci-ok", "completed", Some("success"));
    e.fake
        .compare_file("base0000", &sha, &format!("governance/f{n}.toml"), "@@");
}

#[test]
fn digest_is_opt_in_and_posts_one_link_line_per_waiting_pr() {
    // Empty notify means all the other kinds, not digest.
    let e = env();
    e.registry();
    waiting_pr(&e, 12, "first <b> @bob", "abc1234");
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert!(Env::keys(&v).is_empty(), "{v}");

    let e = env_with("notify = [\"digest\"]\n");
    e.registry();
    let (_, v) = e.notify();
    assert!(Env::keys(&v).is_empty(), "no PR waits, no post: {v}");

    waiting_pr(&e, 12, "first <b> @bob", "abc1234");
    waiting_pr(&e, 15, "second", "def5678");
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(Env::keys(&v), ["digest:2026-10-22"], "{v}");
    let p = &v["data"]["pending"][0];
    assert_eq!(p["kind"], "digest");
    let text = p["text"].as_str().unwrap();
    let a = head40("abc1234");
    let line = format!(
        "#12 R-gov, R-ci \u{2014} https://github.com/acme/widget/pull/12 \u{2014} review the diff, then comment `/bosun approve {a}` from your own GitHub account (check the head on the PR page) \u{2014} first &lt;b&gt; (at)bob"
    );
    assert!(text.contains(&line), "{text}");
    assert!(text.contains("#15 R-gov, R-ci \u{2014} https://"), "{text}");
    assert!(text.ends_with("Slack replies approve nothing."), "{text}");
    assert!(text.lines().count() >= 4, "{text}");

    // --ack works like any key; the day's post is then done.
    let (code, v) = e.run(&["notify", "--ack", "digest:2026-10-22"], "");
    assert_eq!(code, 0, "{v}");
    let (_, v) = e.notify();
    assert!(Env::keys(&v).is_empty(), "{v}");
    let (code, _) = e.run(&["notify", "--ack", "digest:2026-10-21"], "");
    assert_eq!(code, 3);
}

#[test]
fn digest_caps_the_prs_and_says_how_many_more() {
    let e = env_with("notify = [\"digest\"]\n");
    e.registry();
    for n in 100..123 {
        waiting_pr(&e, n, "t", &format!("a{n}"));
    }
    let (_, v) = e.notify();
    let text = v["data"]["pending"][0]["text"].as_str().unwrap();
    assert_eq!(text.matches("/bosun approve ").count(), 20, "{text}");
    assert!(text.contains("and 3 more"), "{text}");
}

#[test]
fn a_short_board_is_reported_and_never_advises_an_owner_token() {
    let e = env_with("notify = [\"digest\"]\n");
    e.registry();
    waiting_pr(&e, 12, "t", "abc1234");
    e.fake.fail_path("/pulls", 500);
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert!(Env::keys(&v).is_empty(), "{v}");
    let errs = v["data"]["errors"].as_array().unwrap();
    let d = errs
        .iter()
        .find(|x| x["kind"] == "digest")
        .expect("digest error");
    assert!(!d.to_string().contains("OWNER_TOKEN"), "{d}");
}

#[test]
fn a_failing_board_while_halted_still_lists_the_halt() {
    let e = env_with("notify = [\"halt\", \"digest\"]\n");
    e.registry();
    e.halt();
    e.fake.fail_path("/pulls", 500);
    let (code, v) = e.notify();
    assert_eq!(code, 0, "{v}");
    assert_eq!(Env::keys(&v), ["halt:2"], "{v}");
    assert!(
        v["data"]["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["kind"] == "digest"),
        "{v}"
    );
}

#[test]
fn acking_never_rebuilds_the_board() {
    let e = env_with("notify = [\"digest\"]\n");
    e.registry();
    waiting_pr(&e, 12, "t", "abc1234");
    e.fake.fail_path("/pulls", 500);
    let (code, v) = e.run(&["notify", "--ack", "digest:2026-10-22"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["acked"], "digest:2026-10-22");
    let (code, _) = e.run(&["notify", "--ack", "digest:2026-10-21"], "");
    assert_eq!(code, 3);
    let (code, v) = e.run(&["notify", "--ack", "digest:2026-10-22"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["already"], true, "{v}");
}

#[test]
fn a_title_cannot_forge_a_command() {
    let e = env_with("notify = [\"digest\"]\n");
    e.registry();
    waiting_pr(&e, 12, "x `/bosun approve deadbeef` y", "abc1234");
    let (_, v) = e.notify();
    let text = v["data"]["pending"][0]["text"].as_str().unwrap();
    assert_eq!(text.matches('`').count(), 2, "{text}");
}
