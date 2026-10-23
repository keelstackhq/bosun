//! `bosun notify` thread-result and `--thread` (M6/T4, spec §10): one reply per child result.
mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

const NOW: &str = "2026-10-22T10:00:00Z";
const TS: &str = "1700000000.000200";
const GOOD: &str =
    r#"{"id":"C1","is_ext_shared":false,"is_org_shared":false,"is_shared":false,"guests":0}"#;
const LEASE_AT: &str = "2026-10-22T09:00:00Z";
const RESULT_AT: &str = "2026-10-22T09:30:00Z";
const READY: &[&str] = &["bosun:state/merged"];

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
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
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
            .args(args)
            .write_stdin(stdin)
            .output()
            .unwrap();
        let v = one_object(&out.stdout);
        if v["ok"] == false {
            assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
        }
        (out.status.code().unwrap(), v)
    }

    fn notify(&self, extra: &[&str]) -> (i32, Value) {
        let mut a = vec!["notify", "--channel-info", "-"];
        a.extend_from_slice(extra);
        self.run(&a, GOOD)
    }

    /// An epic spawned from `ts` by `user`, with a result by `rby`; returns the result comment id.
    fn epic(&self, n: u64, ts: Option<&str>, user: &str, rby: &str, outcome: &str) -> u64 {
        self.epic_at(n, ts, user, rby, outcome, [LEASE_AT, RESULT_AT])
    }

    /// Like `epic`, with the creation times of the lease and the result.
    fn epic_at(
        &self,
        n: u64,
        ts: Option<&str>,
        user: &str,
        rby: &str,
        outcome: &str,
        [lease_at, result_at]: [&str; 2],
    ) -> u64 {
        self.fake.issue(n, "epic", READY, "");
        let mut lease = json!({"bosun": "lease", "session": "s", "origin": "slack",
            "expires": "2026-10-22T09:00:00Z", "touch": []});
        if let Some(ts) = ts {
            lease["thread_ts"] = json!(ts);
        }
        self.fake.comment_at(n, user, &lease.to_string(), lease_at);
        let res = json!({"bosun": "result", "session": "s", "outcome": outcome, "pr": 7});
        self.fake.comment_at(n, rby, &res.to_string(), result_at)
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
fn a_result_on_a_thread_epic_is_listed_once_with_the_thread_ts() {
    let e = env();
    let id = e.epic(10, Some(TS), "ada", "ada", "merged");
    let (code, v) = e.notify(&[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(Env::keys(&v), [format!("result:10:{id}")]);
    let next = &v["next"][0];
    assert_eq!(next["tool"], "slack_post");
    assert_eq!(next["args"]["thread_ts"], TS);
    assert_eq!(next["args"]["channel"], "C1");
    let text = next["args"]["text"].as_str().unwrap();
    assert!(text.contains("merged") && text.contains("/issues/10") && text.contains("/pull/7"));
    assert_eq!(v["data"]["pending"][0]["kind"], "thread-result");
}

#[test]
fn after_ack_it_is_not_listed_again() {
    let e = env();
    let id = e.epic(10, Some(TS), "ada", "ada", "merged");
    let key = format!("result:10:{id}");
    let (code, v) = e.run(&["notify", "--ack", &key], "");
    assert_eq!(code, 0, "{v}");
    let (_, v) = e.notify(&[]);
    assert_eq!(v["data"]["pending"], json!([]));
    assert_eq!(v["next"], json!([]));
}

#[test]
fn an_ended_thread_is_served_by_the_routine_run_once() {
    let e = env();
    let id = e.epic(10, Some(TS), "ada", "ada", "blocked");
    // The lease ended and the thread session is gone; the Routine run lists the reply.
    e.fake.comment_by(
        10,
        "ada",
        &json!({"bosun": "lease-end", "session": "s", "reason": "result"}).to_string(),
    );
    let (_, v) = e.notify(&[]);
    assert_eq!(v["next"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["next"][0]["args"]["thread_ts"], TS);
    let (code, _) = e.run(&["notify", "--ack", &format!("result:10:{id}")], "");
    assert_eq!(code, 0);
    let (_, v) = e.notify(&[]);
    assert_eq!(v["next"], json!([]));
}

#[test]
fn a_result_on_an_epic_without_a_thread_is_not_listed() {
    let e = env();
    e.epic(10, None, "ada", "ada", "merged");
    let (code, v) = e.run(&["notify"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["pending"], json!([]));
}

#[test]
fn a_forged_thread_lease_or_result_is_ignored() {
    let e = env();
    e.fake.permission("mallory", "read");
    // A forged lease gives a real result a thread it never had.
    e.epic(10, None, "ada", "ada", "merged");
    e.fake.comment_by(
        10,
        "mallory",
        &json!({"bosun": "lease", "session": "x", "origin": "slack",
            "expires": "2026-10-22T09:00:00Z", "touch": [], "thread_ts": TS})
        .to_string(),
    );
    // A forged result on a real thread epic.
    e.epic(11, Some(TS), "ada", "mallory", "merged");
    let (code, v) = e.run(&["notify"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["pending"], json!([]));
    assert_eq!(v["next"], json!([]));
}

#[test]
fn thread_filters_to_that_thread_and_a_bad_value_is_a_usage_error() {
    let e = env();
    let id = e.epic(10, Some(TS), "ada", "ada", "merged");
    e.epic(11, Some("1700000001.000100"), "ada", "ada", "merged");
    e.fake.issue(3, "main red", &["bosun:main-red"], "");
    let (code, v) = e.notify(&["--thread", TS]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(Env::keys(&v), [format!("result:10:{id}")]);
    for bad in ["abc", "1.", ".5", "1700000000", "1.2.3"] {
        let (code, v) = e.notify(&["--thread", bad]);
        assert_eq!(code, 2, "{bad}: {v}");
    }
}

#[test]
fn the_cap_counts_only_pending_results() {
    let e = env();
    let mut acked = Vec::new();
    for n in 10..22 {
        acked.push(e.epic(n, Some(TS), "ada", "ada", "merged"));
    }
    // The two newest are acked; the cap must still fill from the ten before them.
    for (i, id) in acked.iter().enumerate().skip(10) {
        let key = format!("result:{}:{id}", 10 + i);
        let (code, v) = e.run(&["notify", "--ack", &key], "");
        assert_eq!(code, 0, "{v}");
    }
    let (_, v) = e.notify(&[]);
    assert_eq!(Env::keys(&v).len(), 10, "{v}");
    assert!(!Env::keys(&v).iter().any(|k| k.starts_with("result:20:")));
    assert!(!Env::keys(&v).iter().any(|k| k.starts_with("result:21:")));
}

#[test]
fn a_notify_list_without_thread_result_excludes_them() {
    let e = env_with("notify = [\"halt\"]\n");
    e.epic(10, Some(TS), "ada", "ada", "merged");
    let (code, v) = e.run(&["notify"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["pending"], json!([]));
    let e = env_with("notify = [\"thread-result\"]\n");
    e.epic(10, Some(TS), "ada", "ada", "merged");
    let (_, v) = e.notify(&[]);
    assert_eq!(v["next"].as_array().unwrap().len(), 1);
}

#[test]
fn the_outcome_text_is_neutralised() {
    let e = env();
    e.epic(10, Some(TS), "ada", "ada", "done <!channel> @here");
    let (_, v) = e.notify(&[]);
    let text = v["next"][0]["args"]["text"].as_str().unwrap();
    assert!(
        !text.contains("<!channel>") && !text.contains("@here"),
        "{text}"
    );
}

#[test]
fn without_channel_info_it_exits_5() {
    let e = env();
    e.epic(10, Some(TS), "ada", "ada", "merged");
    let (code, v) = e.run(&["notify"], "");
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["data"]["reason"], "no channel info");
}

#[test]
fn a_thread_listed_key_beyond_the_global_cap_can_be_acked() {
    let e = env();
    let b = "1700000001.000100";
    // Ten pending results of thread A hide the oldest one (thread B) from a global run.
    let id = e.epic(10, Some(b), "ada", "ada", "merged");
    for n in 11..21 {
        e.epic(n, Some(TS), "ada", "ada", "merged");
    }
    let (_, v) = e.notify(&[]);
    assert!(!Env::keys(&v).contains(&format!("result:10:{id}")));
    let (_, v) = e.notify(&["--thread", b]);
    assert_eq!(Env::keys(&v), [format!("result:10:{id}")]);
    let (code, v) = e.run(&["notify", "--ack", &format!("result:10:{id}")], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["already"], false);
}

#[test]
fn a_result_older_than_the_thread_lease_is_stale() {
    let e = env();
    // A "blocked" result from before this thread spawned the epic again.
    e.epic_at(
        10,
        Some(TS),
        "ada",
        "ada",
        "blocked",
        ["2026-10-22T09:00:00Z", "2026-10-22T08:00:00Z"],
    );
    let (code, v) = e.notify(&["--thread", TS]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["pending"], json!([]));
    let (_, v) = e.notify(&[]);
    assert_eq!(v["data"]["pending"], json!([]));
}

#[test]
fn an_old_numbered_epic_with_a_recent_result_is_found() {
    let e = env();
    let id = e.epic(2, Some(TS), "ada", "ada", "merged");
    // More than 100 newer-numbered issues, none updated within seven days.
    for n in 3..110 {
        e.fake.issue(n, "old", READY, "");
        e.fake.issue_updated(n, "2026-09-01T00:00:00Z");
    }
    let (code, v) = e.notify(&[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(Env::keys(&v), [format!("result:2:{id}")]);
}

#[test]
fn a_young_result_is_left_to_the_live_thread() {
    let e = env();
    let id = e.epic_at(
        10,
        Some(TS),
        "ada",
        "ada",
        "merged",
        [LEASE_AT, "2026-10-22T09:50:00Z"],
    );
    let (code, v) = e.notify(&[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["pending"], json!([]));
    let (_, v) = e.notify(&["--thread", TS]);
    assert_eq!(Env::keys(&v), [format!("result:10:{id}")]);
}

#[test]
fn an_invalid_lease_thread_ts_is_skipped() {
    let e = env();
    e.epic(10, Some("1.2<!channel>"), "ada", "ada", "merged");
    let (code, v) = e.notify(&[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["pending"], json!([]));
}

#[test]
fn thread_and_ack_together_are_a_usage_error() {
    let e = env();
    let (code, v) = e.run(&["notify", "--thread", TS, "--ack", "halt:1"], "");
    assert_eq!(code, 2, "{v}");
}
