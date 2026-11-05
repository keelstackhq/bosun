mod common;

use common::fake::FakeGithub;
use common::{lease_note, run};

const NOW: &str = "2026-10-22T11:00:00Z";
const LIVE: &str = "2026-10-22T11:20:00Z";
const BUILDING: &str = "bosun:state/building";
const READY: &str = "bosun:state/ready";

fn join_note(session: &str, parent: Option<&str>) -> String {
    let parent = parent.map_or(String::new(), |p| format!(",\"parent\":\"{p}\""));
    format!(
        "{{\"bosun\":\"join\",\"session\":\"{session}\",\"origin\":\"local\"{parent},\"depth\":1,\"expires\":\"{LIVE}\"}}"
    )
}

fn handoff_note(session: &str, line: &str) -> String {
    format!(
        "{{\"bosun\":\"handoff\",\"session\":\"{session}\",\"at\":\"{NOW}\",\"lines\":[\"{line}\"]}}"
    )
}

fn registry(fake: &FakeGithub) {
    fake.issue(1, "bosun registry", &["bosun:registry"], "");
}

fn bodies(fake: &FakeGithub, issue: u64) -> Vec<String> {
    fake.comments_of(issue)
        .into_iter()
        .map(|c| c.body)
        .collect()
}

fn lines_file(name: &str, text: &str) -> String {
    let path = std::env::temp_dir().join(format!("bosun-handoff-{}-{name}", std::process::id()));
    std::fs::write(&path, text).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn handoff_writes_open_prs_leases_and_the_file_lines() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("root_a", None));
    fake.issue(10, "the task", &[BUILDING], "");
    fake.comment(10, &lease_note("root_a", LIVE, "src/core/**"));
    fake.pull(20, "feat: the task", "claude/the-task", "Closes #10");
    fake.issue(12, "other task", &[BUILDING], "");
    fake.comment(12, &lease_note("other_s", LIVE, "docs/**"));
    fake.pull(30, "other", "claude/other", "Closes #12");
    fake.pull(31, "unrelated", "claude/unrelated", "");
    fake.issue(13, "wave task", &[BUILDING], "");
    fake.comment(13, &lease_note("root_a/T7", LIVE, "tests/**"));
    fake.pull(32, "wave", "claude/wave", "Closes #13");
    fake.issue(14, "lookalike", &[BUILDING], "");
    fake.comment(14, &lease_note("root_ab", LIVE, "x/**"));
    let file = lines_file("ok", "next: finish #10\n\nblocker: none\n");

    let (code, v) = run(&fake, "root_a", NOW, &["handoff", "--from", &file]);
    assert_eq!(code, 0, "{v}");
    let lines: Vec<&str> = v["data"]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    assert_eq!(lines[0], "pr: #20 claude/the-task sha20");
    assert_eq!(lines[1], "pr: #32 claude/wave sha32");
    assert!(lines[2].starts_with("lease: #10 root_a src/core/** until 2026-10-22T11:20:00Z"));
    assert!(lines[3].starts_with("lease: #13 root_a/T7 tests/** until"));
    assert_eq!(&lines[4..], ["next: finish #10", "blocker: none"]);
    assert_eq!(v["data"]["count"], 6);
    let all = bodies(&fake, 1);
    assert_eq!(all.len(), 2);
    assert!(
        all[1].contains(r#"{"bosun":"handoff","session":"root_a""#),
        "{}",
        all[1]
    );
    assert!(all[1].contains("sha20"));
}

#[test]
fn uc_zh_1_more_than_40_lines_is_exit_6_and_writes_nothing() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("root_a", None));
    let text: String = (0..41).map(|i| format!("next: step {i}\n")).collect();
    let before = bodies(&fake, 1);

    let (code, v) = run(
        &fake,
        "root_a",
        NOW,
        &["handoff", "--from", &lines_file("long", &text)],
    );
    assert_eq!(code, 6, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert_eq!(v["data"]["lines"], 41);
    assert_eq!(bodies(&fake, 1), before);

    let ok: String = (0..40).map(|i| format!("next: step {i}\n")).collect();
    let file = lines_file("forty", &ok);
    let (code, v) = run(&fake, "root_a", NOW, &["handoff", "--from", &file]);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn uc_zh_2_resume_handoff_returns_the_newest_of_the_root_lineage() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("root_a", None));
    fake.comment(1, &handoff_note("root_a", "next: old"));
    fake.comment(1, &handoff_note("root_a", "next: new"));
    let (code, v) = run(&fake, "root_b", NOW, &["resume", "--handoff"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["handoff"]["lines"][0], "next: new");
    assert_eq!(v["data"]["handoff"]["session"], "root_a");
    assert_eq!(v["data"]["joined"], true);
    assert!(v["data"]["tasks"].is_array());
}

#[test]
fn resume_handoff_ignores_a_newer_handoff_of_a_child() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("root_a", None));
    fake.comment(1, &join_note("child_c", Some("root_a")));
    fake.comment(1, &handoff_note("root_a", "next: root"));
    fake.comment(1, &handoff_note("child_c", "next: child"));
    let (code, v) = run(&fake, "root_b", NOW, &["resume", "--handoff"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["handoff"]["lines"][0], "next: root");
}

#[test]
fn resume_handoff_without_a_handoff_is_null_and_plain_resume_has_no_key() {
    let fake = FakeGithub::start();
    registry(&fake);
    let (_, v) = run(&fake, "root_b", NOW, &["resume", "--handoff"]);
    assert!(v["data"]["handoff"].is_null());
    assert!(v["data"].as_object().unwrap().contains_key("handoff"));
    let (_, v) = run(&fake, "root_b", NOW, &["resume"]);
    assert!(!v["data"].as_object().unwrap().contains_key("handoff"));
}

#[test]
fn uc_zh_3_a_handoff_never_creates_or_renews_a_lease() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("root_a", None));
    fake.issue(10, "the task", &[BUILDING], "");
    fake.comment(
        10,
        &lease_note("root_a", "2026-10-22T11:05:00Z", "src/core/**"),
    );
    fake.issue(11, "other", &[READY], "");
    let (reg, task, other) = (bodies(&fake, 1), bodies(&fake, 10), bodies(&fake, 11));

    let file = lines_file("lease", "next: go on\n");
    let (code, v) = run(&fake, "root_a", NOW, &["handoff", "--from", &file]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(bodies(&fake, 10), task, "lease comment untouched");
    assert_eq!(bodies(&fake, 11), other);
    let after = bodies(&fake, 1);
    assert_eq!(after[..reg.len()], reg[..], "join record untouched");
    assert_eq!(after.len(), reg.len() + 1);
    assert!(!after[reg.len()].contains(r#""bosun":"lease""#));
}

#[test]
fn a_bad_kind_in_the_file_is_a_usage_error() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("root_a", None));
    let file = lines_file("bad", "next: ok\ntodo: nope\n");
    let (code, v) = run(&fake, "root_a", NOW, &["handoff", "--from", &file]);
    assert_eq!(code, 2, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("line 2"));
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert_eq!(bodies(&fake, 1).len(), 1);
}

#[test]
fn a_child_session_cannot_hand_off() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("child_c", Some("root_a")));
    let (code, v) = run(&fake, "child_c", NOW, &["handoff"]);
    assert_eq!(code, 1, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert_eq!(bodies(&fake, 1).len(), 1);
}

fn issue_count(fake: &FakeGithub) -> usize {
    fake.state.lock().unwrap().issues.len()
}

#[test]
fn a_forged_handoff_from_a_user_without_write_access_is_ignored() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("root_a", None));
    fake.comment_by(1, "owner", &handoff_note("root_a", "next: real"));
    fake.permission("mallory", "read");
    fake.comment_by(1, "mallory", &handoff_note("root_a", "next: forged"));
    let (code, v) = run(&fake, "root_b", NOW, &["resume", "--handoff"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["handoff"]["lines"][0], "next: real");

    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("root_a", None));
    fake.permission("mallory", "read");
    fake.comment_by(1, "mallory", &handoff_note("root_a", "next: forged"));
    let (_, v) = run(&fake, "root_b", NOW, &["resume", "--handoff"]);
    assert!(v["data"]["handoff"].is_null());
}

#[test]
fn a_malformed_stored_handoff_is_skipped() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("root_a", None));
    fake.comment(1, &handoff_note("root_a", "next: good"));
    fake.comment(1, &handoff_note("root_a", "todo: bad kind"));
    let (_, v) = run(&fake, "root_b", NOW, &["resume", "--handoff"]);
    assert_eq!(v["data"]["handoff"]["lines"][0], "next: good");
}

#[test]
fn built_lines_are_cleaned_of_hidden_characters() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("root_a", None));
    fake.pull(20, "feat", "claude/a\u{202e}b", "Closes #10");
    fake.issue(10, "the task", &[BUILDING], "");
    fake.comment(10, &lease_note("root_a", LIVE, "src/**"));
    let (code, v) = run(&fake, "root_a", NOW, &["handoff"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["lines"][0], "pr: #20 claude/a b sha20");
}

#[test]
fn a_child_who_joined_and_left_still_cannot_hand_off_and_is_ignored() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("child_c", Some("root_a")));
    fake.comment(
        1,
        r#"{"bosun":"leave","session":"child_c","reason":"done"}"#,
    );
    let (code, v) = run(&fake, "child_c", NOW, &["handoff"]);
    assert_eq!(code, 1, "{v}");
    fake.comment(1, &handoff_note("child_c", "next: child"));
    let (_, v) = run(&fake, "root_b", NOW, &["resume", "--handoff"]);
    assert!(v["data"]["handoff"].is_null());
}

#[test]
fn a_spawned_origin_cannot_hand_off() {
    let fake = FakeGithub::start();
    registry(&fake);
    let out = common::bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_ORIGIN", "spawned")
        .arg("handoff")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn refusals_write_no_comment() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("root_a", None));
    let long: String = (0..41).map(|i| format!("next: s{i}\n")).collect();
    let cases = [
        (lines_file("l41", &long), 6),
        (lines_file("badk", "oops\n"), 2),
        ("/nonexistent/h.txt".to_string(), 2),
    ];
    for (file, want) in cases {
        let (code, v) = run(&fake, "root_a", NOW, &["handoff", "--from", &file]);
        assert_eq!(code, want, "{v}");
    }
    assert_eq!(bodies(&fake, 1).len(), 1);
    assert_eq!(issue_count(&fake), 1);
}

#[test]
fn own_prs_and_leases_count_toward_the_limit() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("root_a", None));
    for n in [10, 11] {
        fake.issue(n, "t", &[BUILDING], "");
        fake.comment(n, &lease_note("root_a", LIVE, "src/**"));
        fake.pull(
            n + 10,
            "t",
            &format!("claude/t{n}"),
            &format!("Closes #{n}"),
        );
    }
    let text: String = (0..37).map(|i| format!("next: s{i}\n")).collect();
    let (code, _) = run(
        &fake,
        "root_a",
        NOW,
        &["handoff", "--from", &lines_file("p37", &text)],
    );
    assert_eq!(code, 6);
    assert_eq!(bodies(&fake, 1).len(), 1);
}

#[test]
fn from_dash_reads_stdin() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("session_test", None));
    let out = common::bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .args(["handoff", "--from", "-"])
        .write_stdin("next: from stdin\n")
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{v}");
    assert_eq!(v["data"]["lines"][0], "next: from stdin");
}

#[test]
fn a_wave_agent_id_is_refused_and_never_read_back() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("root_a/T7", None));
    let (code, v) = run(&fake, "root_a/T7", NOW, &["handoff"]);
    assert_eq!(code, 1, "{v}");
    fake.comment(1, &handoff_note("root_a/T7", "next: wave agent"));
    let (_, v) = run(&fake, "root_b", NOW, &["resume", "--handoff"]);
    assert!(v["data"]["handoff"].is_null());
}

#[test]
fn an_unjoined_session_is_refused_and_never_read_back() {
    let fake = FakeGithub::start();
    registry(&fake);
    let (code, v) = run(&fake, "loner", NOW, &["handoff"]);
    assert_eq!(code, 1, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("bosun join"));
    assert_eq!(bodies(&fake, 1).len(), 0);
    fake.comment(1, &handoff_note("loner", "next: x"));
    let (_, v) = run(&fake, "root_b", NOW, &["resume", "--handoff"]);
    assert!(v["data"]["handoff"].is_null());
}

#[test]
fn a_joined_root_with_spawned_origin_is_not_read_back() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(
        1,
        &join_note("root_a", None).replace("\"local\"", "\"spawned\""),
    );
    fake.comment(1, &handoff_note("root_a", "next: x"));
    let (_, v) = run(&fake, "root_b", NOW, &["resume", "--handoff"]);
    assert!(v["data"]["handoff"].is_null());
}

#[test]
fn a_secret_in_the_input_is_refused_and_writes_nothing() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("root_a", None));
    let secret = format!("sk-ant-{}", "a1B2".repeat(9));
    let file = lines_file("secret", &format!("next: ok\nblocker: key is {secret}\n"));
    let (code, v) = run(&fake, "root_a", NOW, &["handoff", "--from", &file]);
    assert_eq!(code, 2, "{v}");
    assert!(v["error"]["fix"].as_str().unwrap().contains("redact"));
    let message = v["error"]["message"].as_str().unwrap();
    assert!(message.contains("API key"), "{message}");
    assert!(!v.to_string().contains(&secret));
    assert_eq!(bodies(&fake, 1).len(), 1, "no handoff comment");
}

#[test]
fn a_session_id_in_a_line_is_not_a_secret() {
    let fake = FakeGithub::start();
    registry(&fake);
    fake.comment(1, &join_note("root_a", None));
    let file = lines_file(
        "sid",
        &format!(
            "next: ask session_{} about it\n",
            "01SsFUkfeX2mg29KV4Lhoi5P"
        ),
    );
    let (code, v) = run(&fake, "root_a", NOW, &["handoff", "--from", &file]);
    assert_eq!(code, 0, "{v}");
}
