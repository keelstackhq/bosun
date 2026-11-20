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
            "acme/widget#{pr} `t` (risk R-gov; verdict: ok; head {s} = {h})",
            s = &h[..7]
        )
    };
    let mark = |pr: u64, h: &str| format!("[bosun:{pr}-{}-1792663200]", &h[..7]);
    let text = match items {
        [(pr, h)] => format!("Approve {}? {}", line(*pr, h), mark(*pr, h)),
        many => {
            let lines: Vec<String> = many
                .iter()
                .map(|(p, h)| format!("- {} {}", line(*p, h), mark(*p, h)))
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
    json!({"session_id": "s", "tool_use_id": "toolu_1", "hook_event_name": "PostToolUse",
        "tool_name": "AskUserQuestion",
        "tool_input": {"questions": [q]},
        "tool_response": {"questions": [q], "answers": {text: answer}}})
}

fn post(repo: &Path, input: &Value) -> std::process::Output {
    post_at(repo, input, NOW)
}

/// The session transcript for `input`: the assistant's AskUserQuestion `tool_use` and (unless
/// `result` is false) its `tool_result`, as JSONL under the repo's `.git`.
fn transcript(repo: &Path, input: &Value, result: bool) -> Value {
    transcript_as(
        repo,
        input,
        result,
        input["tool_response"]["answers"].clone(),
        input["session_id"].as_str().unwrap_or("s"),
    )
}

/// The same, with `answers` as the transcript's tool_result says them, under the file name `<name>.jsonl`.
fn transcript_as(repo: &Path, input: &Value, result: bool, answers: Value, name: &str) -> Value {
    let id = &input["tool_use_id"];
    let said: String = (answers.as_object().into_iter().flatten())
        .map(|(q, a)| format!("\"{q}\"=\"{}\"", a.as_str().unwrap_or("")))
        .collect::<Vec<_>>()
        .join(", ");
    let sid = input["session_id"].clone();
    let call = json!({"type": "assistant", "sessionId": sid, "message": {"role": "assistant", "content": [
        {"type": "tool_use", "id": id, "name": "AskUserQuestion", "input": input["tool_input"]}]}});
    // the real format: a text `tool_result` plus the structured `toolUseResult.answers`
    let done = json!({"type": "user", "sessionId": sid, "toolUseResult": {"answers": answers},
        "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": id,
            "content": format!("Your questions have been answered: {said}. You can now continue with these answers in mind.")}]}});
    let lines = if result { vec![call, done] } else { vec![call] };
    let dir = repo
        .join(".home")
        .join(".claude")
        .join("projects")
        .join("p");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("{name}.jsonl"));
    let text: Vec<String> = lines.iter().map(Value::to_string).collect();
    std::fs::write(&file, text.join("\n")).unwrap();
    json!(file)
}

fn with_transcript(repo: &Path, mut input: Value) -> Value {
    input["transcript_path"] = transcript(repo, &input, true);
    input
}

fn post_at(repo: &Path, input: &Value, now: &str) -> std::process::Output {
    let mut input = input.clone();
    if input.is_object() && input.get("transcript_path").is_none() {
        input = with_transcript(repo, input);
    }
    post_raw(repo, &input, now)
}

fn post_raw(repo: &Path, input: &Value, now: &str) -> std::process::Output {
    bosun()
        .current_dir(repo)
        .env("BOSUN_NOW", now)
        .env("HOME", repo.join(".home"))
        .env_remove("CLAUDE_CODE_CHILD_SESSION")
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
    assert_eq!(a["repo"], json!("acme/widget"));
    assert_eq!(a["pr"], json!(12));
    assert_eq!(a["head"], json!(h));
    assert_eq!(a["at"], json!(NOW));
    assert_eq!(a["cli_session_id"], json!("s"));
    assert!(a["session"].as_str().is_some_and(|s| !s.is_empty()), "{a}");
    assert_eq!(a["tool_use_id"], json!("toolu_1"));
}

#[test]
fn answers_only_in_tool_input_store_nothing() {
    let r = repo();
    let mut input = question(&[(12, &head("abc1234"))], "Hold");
    let forged = input["tool_response"]["answers"].clone();
    input["tool_response"] = json!({});
    input["tool_input"]["answers"] = forged;
    post(r.path(), &input);
    assert_eq!(stored(r.path()), json!({}));
}

#[test]
fn other_tool_stores_nothing() {
    let r = repo();
    let mut input = question(&[(12, &head("abc1234"))], "Hold");
    input["tool_name"] = json!("Bash");
    post(r.path(), &input);
    assert!(!answers_file(r.path()).exists());
}

#[test]
fn decoy_head_after_the_real_one_is_not_taken() {
    let r = repo();
    let (h, other) = (head("abc1234"), head("abc1234f"));
    let text = format!(
        "Approve acme/widget#12 `t` (verdict: head abc1234 = {other}; head abc1234 = {h})? \
         [bosun:12-abc1234-1792663200]"
    );
    let mut input = question(&[(12, &h)], "Approve #12 abc1234");
    input["tool_input"]["questions"][0]["question"] = json!(text);
    input["tool_response"]["answers"] = json!({text: "Approve #12 abc1234"});
    post(r.path(), &input);
    assert_eq!(stored(r.path())["12-abc1234-1792663200"]["head"], json!(h));
    // text after the head, before the marker, binds nothing
    let text = format!(
        "Approve acme/widget#12 `t` (verdict: ok; head abc1234 = {h}) head abc1234 = {other} \
         [bosun:13-abc1234-1792663200]"
    );
    let mut input = question(&[(13, &h)], "Approve #13 abc1234");
    input["tool_input"]["questions"][0]["question"] = json!(text);
    input["tool_response"]["answers"] = json!({text: "Approve #13 abc1234"});
    post(r.path(), &input);
    assert!(stored(r.path()).get("13-abc1234-1792663200").is_none());
}

#[test]
fn pr_number_must_be_plain_digits() {
    let r = repo();
    let h = head("abc1234");
    for pr in ["+12", "012", "-1"] {
        let text = format!(
            "Approve acme/widget#12 `t` (verdict: ok; head abc1234 = {h})? \
             [bosun:{pr}-abc1234-1792663200]"
        );
        let mut input = question(&[(12, &h)], "Hold");
        input["tool_input"]["questions"][0]["question"] = json!(text);
        input["tool_response"]["answers"] = json!({text: "Hold"});
        post(r.path(), &input);
    }
    assert_eq!(stored(r.path()), json!({}));
}

#[test]
fn newest_answer_wins_between_hold_and_approve() {
    let r = repo();
    let fake = FakeGithub::start();
    let h = head("abc1234");
    ready(&fake, 12, &h);
    let (t1, t2, t3) = (
        "2026-10-22T10:00:00Z",
        "2026-10-22T11:00:00Z",
        "2026-10-22T12:00:00Z",
    );
    post_at(r.path(), &question(&[(12, &h)], "Hold"), t1);
    let approve = later(question(&[(12, &h)], "Approve #12 abc1234"), "1792663201");
    post_at(r.path(), &approve, t2);
    assert_eq!(list_at(&fake, r.path(), t2)["data"]["held"], json!([]));
    let hold = later(question(&[(12, &h)], "Hold"), "1792663202");
    post_at(r.path(), &hold, t3);
    assert_eq!(
        list_at(&fake, r.path(), t3)["data"]["held"],
        json!([{"pr": 12, "head_sha": h}])
    );
}

#[test]
fn a_future_dated_answer_never_outranks_a_later_one_and_is_pruned() {
    let r = repo();
    let fake = FakeGithub::start();
    let h = head("abc1234");
    ready(&fake, 12, &h);
    // A hold stored by the hook now, then an approve hand-written into the file a year ahead.
    post_at(r.path(), &question(&[(12, &h)], "Hold"), NOW);
    let mut s = stored(r.path());
    s["12-abc1234-1"] = json!({"answer": "approve", "repo": "acme/widget", "pr": 12,
        "head": h, "at": "2027-10-22T10:00:00Z", "session": "session_x",
        "cli_session_id": "c", "tool_use_id": "t"});
    std::fs::write(answers_file(r.path()), s.to_string()).unwrap();
    // The future approve does not count: the hold stays the newest answer.
    assert_eq!(
        list(&fake, r.path())["data"]["held"],
        json!([{"pr": 12, "head_sha": h}])
    );
    // The next write prunes it.
    post_at(r.path(), &question(&[(13, &head("def5678"))], "Hold"), NOW);
    assert!(stored(r.path()).get("12-abc1234-1").is_none());
}

#[test]
fn write_prunes_old_entries_and_other_heads_of_the_same_pr() {
    let r = repo();
    let (a, b, c) = (head("aaaaaaa"), head("bbbbbbb"), head("ccccccc"));
    post_at(
        r.path(),
        &question(&[(1, &a)], "Hold"),
        "2026-10-01T10:00:00Z",
    );
    post_at(
        r.path(),
        &question(&[(2, &b)], "Hold"),
        "2026-10-20T10:00:00Z",
    );
    post_at(
        r.path(),
        &question(&[(3, &c)], "Hold"),
        "2026-10-22T10:00:00Z",
    );
    let s = stored(r.path());
    assert!(
        s.get("1-aaaaaaa-1792663200").is_none(),
        "older than 14 days: {s}"
    );
    assert!(s.get("2-bbbbbbb-1792663200").is_some(), "{s}");
    let b2 = head("ddddddd");
    post_at(
        r.path(),
        &question(&[(2, &b2)], "Hold"),
        "2026-10-22T11:00:00Z",
    );
    let s = stored(r.path());
    assert!(
        s.get("2-bbbbbbb-1792663200").is_none(),
        "other head of PR 2: {s}"
    );
    assert!(s.get("2-ddddddd-1792663200").is_some());
    assert!(s.get("3-ccccccc-1792663200").is_some());
}

/// The same input with another unix stamp in every question id.
fn later(q: Value, unix: &str) -> Value {
    serde_json::from_str(&q.to_string().replace("1792663200", unix)).unwrap()
}

fn pre(repo: &Path, tool: &str, tool_input: Value) -> String {
    let input = json!({"session_id": "w", "tool_name": tool, "tool_input": tool_input});
    let out = bosun()
        .current_dir(repo)
        .env("BOSUN_NOW", NOW)
        .args(["hook", "pre-tool-use"])
        .write_stdin(input.to_string())
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn workers_cannot_write_the_answers_file_or_run_the_hook() {
    let r = repo();
    let file = answers_file(r.path()).display().to_string();
    for tool in ["Edit", "Write", "MultiEdit"] {
        let out = pre(r.path(), tool, json!({"file_path": file, "content": "{}"}));
        assert!(out.contains("\"deny\""), "{tool}: {out}");
    }
    let out = pre(r.path(), "NotebookEdit", json!({"notebook_path": file}));
    assert!(out.contains("\"deny\""), "{out}");
    for cmd in [
        format!("echo '{{}}' > {file}"),
        "cp x .git/bosun/answers.json".to_string(),
        "mv x .git/bosun/ANSWERS.json".to_string(),
        "ln -s x .git/bosun/answers.json".to_string(),
        "rsync x .git/bosun/answers.json".to_string(),
        "cp -t .git/bosun x".to_string(),
        "cd .git/bosun && cp x answers.json".to_string(),
        "python3 -c \"open('answers.json','w')\"".to_string(),
    ] {
        let out = pre(r.path(), "Bash", json!({"command": cmd}));
        assert!(out.contains("\"deny\""), "{cmd}: {out}");
    }
    for cmd in [
        "ls".to_string(),
        "cat .git/bosun/answers.json".to_string(),
        "cat .git/bosun/answers.json 2>/dev/null".to_string(),
        "test -f .git/bosun/answers.json".to_string(),
        "python3 -m json.tool .git/bosun/answers.json".to_string(),
        "git commit -m 'hook post-tool-use'".to_string(),
        "grep -rn post-tool-use src".to_string(),
    ] {
        let out = pre(r.path(), "Bash", json!({"command": cmd}));
        assert!(!out.contains("\"deny\""), "{cmd}: {out}");
    }
}

#[test]
fn forged_hook_input_without_a_transcript_entry_stores_nothing() {
    let r = repo();
    let h = head("abc1234");
    let input = question(&[(12, &h)], "Approve #12 abc1234");
    // no transcript_path, a missing file, a transcript with another tool_use_id, no tool_result
    let mut other = input.clone();
    other["transcript_path"] = transcript(r.path(), &input, true);
    other["tool_use_id"] = json!("toolu_forged");
    let mut open = input.clone();
    open["transcript_path"] = transcript(r.path(), &input, false);
    let mut gone = input.clone();
    gone["transcript_path"] = json!(r.path().join(".git").join("nope.jsonl"));
    for forged in [input.clone(), other, open, gone] {
        let out = post_raw(r.path(), &forged, NOW);
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("not stored"),
            "{out:?}"
        );
    }
    // the transcript asked about another PR head: the marker differs
    let mut swapped = question(&[(12, &head("abc1234"))], "Approve #12 abc1234");
    swapped["transcript_path"] = transcript(r.path(), &question(&[(13, &h)], "x"), true);
    post_raw(r.path(), &swapped, NOW);
    assert_eq!(stored(r.path()), json!({}));
}

#[test]
fn a_symlinked_transcript_stores_nothing() {
    let r = repo();
    let input = question(&[(12, &head("abc1234"))], "Hold");
    let real = transcript(r.path(), &input, true);
    let link = r.path().join(".home/.claude/projects/p/link.jsonl");
    std::os::unix::fs::symlink(real.as_str().unwrap(), &link).unwrap();
    let mut forged = input.clone();
    forged["transcript_path"] = json!(link);
    post_raw(r.path(), &forged, NOW);
    assert_eq!(stored(r.path()), json!({}));
    post(r.path(), &input);
    assert!(stored(r.path()).get("12-abc1234-1792663200").is_some());
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
    list_at(fake, repo, NOW)
}

/// `approve --list` run at `now` (answers dated after `now` never count).
fn list_at(fake: &FakeGithub, repo: &Path, now: &str) -> Value {
    let out = bosun()
        .current_dir(repo)
        .env("BOSUN_NOW", now)
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

#[test]
fn look_alike_marker_stores_nothing() {
    let r = repo();
    let h = head("abc1234");
    for variant in [
        "[BOSUN:12-abc1234-1792663200]",
        "\u{ff3b}bosun:12-abc1234-1792663200]",
    ] {
        let mut input = question(&[(12, &h)], "Approve #12 abc1234");
        let text =
            format!("Approve acme/widget#12 `t` (verdict: ok; head abc1234 = {h})? {variant}");
        input["tool_input"]["questions"][0]["question"] = json!(text);
        input["tool_response"]["answers"] = json!({text: "Approve #12 abc1234"});
        post(r.path(), &input);
    }
    assert_eq!(stored(r.path()), json!({}));
}

#[test]
fn an_empty_session_id_stores_nothing() {
    let r = repo();
    let h = head("abc1234");
    let mut input = question(&[(12, &h)], "Approve #12 abc1234");
    input["session_id"] = json!("");
    let out = post(r.path(), &input);
    assert_eq!(out.status.code(), Some(0));
    assert!(!answers_file(r.path()).exists());
    assert!(String::from_utf8_lossy(&out.stderr).contains("session_id"));
}

#[test]
fn an_answer_for_another_repo_does_not_hold() {
    let r = repo();
    let fake = FakeGithub::start();
    let h = head("abc1234");
    ready(&fake, 12, &h);
    // The hook ran for another repo: the same PR number and head here is not held.
    let out = bosun()
        .current_dir(r.path())
        .env("BOSUN_NOW", NOW)
        .env("BOSUN_REPO", "other/thing")
        .env("HOME", r.path().join(".home"))
        .args(["hook", "post-tool-use"])
        .write_stdin(with_transcript(r.path(), question(&[(12, &h)], "Hold")).to_string())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        stored(r.path())["12-abc1234-1792663200"]["repo"],
        json!("other/thing")
    );
    let v = list(&fake, r.path());
    assert_eq!(v["data"]["held"], json!([]), "{v}");
    assert_eq!(v["data"]["ready"][0]["pr"], json!(12), "{v}");
}

#[test]
fn a_corrupt_answers_file_is_not_used_and_not_reset() {
    let r = repo();
    let fake = FakeGithub::start();
    let h = head("abc1234");
    post(r.path(), &question(&[(12, &h)], "Hold"));
    let file = answers_file(r.path());
    std::fs::write(&file, "{not json").unwrap();
    let out = post(r.path(), &question(&[(13, &head("def5678"))], "Hold"));
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stderr).contains("corrupt"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{not json");
    // Not used: the hold is ignored, so the PR is asked again (fails closed for approvals).
    ready(&fake, 12, &h);
    let v = list(&fake, r.path());
    assert_eq!(v["data"]["held"], json!([]), "{v}");
}

#[test]
fn the_stored_session_is_the_bosun_identity_not_the_cli_uuid() {
    let r = repo();
    let h = head("abc1234");
    let mut input = question(&[(12, &h)], "Approve #12 abc1234");
    input["session_id"] = json!("3f2c9a4e-1b7d-4c55-9a0e-5d6f7a8b9c0d");
    let out = bosun()
        .current_dir(r.path())
        .env("BOSUN_NOW", NOW)
        .env("BOSUN_SESSION", "session_orch")
        .env("HOME", r.path().join(".home"))
        .args(["hook", "post-tool-use"])
        .write_stdin(with_transcript(r.path(), input).to_string())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let a = &stored(r.path())["12-abc1234-1792663200"];
    assert_eq!(a["session"], json!("session_orch"), "{a}");
    assert_eq!(
        a["cli_session_id"],
        json!("3f2c9a4e-1b7d-4c55-9a0e-5d6f7a8b9c0d")
    );
}

#[test]
fn the_transcript_is_pinned_and_must_agree_with_the_hook_input() {
    let r = repo();
    let input = question(&[(12, &head("abc1234"))], "Approve #12 abc1234");
    let text = input["tool_input"]["questions"][0]["question"].clone();
    // a fake transcript elsewhere, saying Hold, while stdin says Approve
    let fake = std::env::temp_dir().join(format!("fake{}.jsonl", std::process::id()));
    let held = json!({text.as_str().unwrap(): "Hold"});
    let real = transcript_as(r.path(), &input, true, held.clone(), "s");
    std::fs::copy(real.as_str().unwrap(), &fake).unwrap();
    let mut forged = input.clone();
    forged["transcript_path"] = json!(fake);
    post_raw(r.path(), &forged, NOW);
    assert_eq!(
        stored(r.path()),
        json!({}),
        "path outside ~/.claude/projects"
    );
    std::fs::remove_file(&fake).unwrap();
    // inside the pinned dir, but the transcript says Hold: disagreement stores nothing
    forged["transcript_path"] = real;
    post_raw(r.path(), &forged, NOW);
    assert_eq!(stored(r.path()), json!({}), "stdin and transcript disagree");
    // the file name must be <session_id>.jsonl
    let named = transcript_as(
        r.path(),
        &input,
        true,
        input["tool_response"]["answers"].clone(),
        "other",
    );
    forged["transcript_path"] = named;
    post_raw(r.path(), &forged, NOW);
    assert_eq!(stored(r.path()), json!({}), "wrong file name");
    post(r.path(), &input);
    assert!(stored(r.path()).get("12-abc1234-1792663200").is_some());
}

#[test]
fn a_child_session_stores_nothing() {
    let r = repo();
    let input = with_transcript(r.path(), question(&[(12, &head("abc1234"))], "Hold"));
    let out = bosun()
        .current_dir(r.path())
        .env("HOME", r.path().join(".home"))
        .env("CLAUDE_CODE_CHILD_SESSION", "yes")
        .env("BOSUN_SESSION", "session_orch")
        .args(["hook", "post-tool-use"])
        .write_stdin(input.to_string())
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("not stored"),
        "{out:?}"
    );
    assert_eq!(stored(r.path()), json!({}));
}

#[test]
fn the_transcript_lines_must_carry_the_input_session_id() {
    let r = repo();
    let input = question(&[(12, &head("abc1234"))], "Approve #12 abc1234");
    let path = transcript(r.path(), &input, true);
    let file = std::path::PathBuf::from(path.as_str().unwrap());
    let good = std::fs::read_to_string(&file).unwrap();
    let other = input["session_id"].as_str().unwrap();
    for bad in [format!("\"sessionId\":\"{other}\""), String::new()] {
        let swapped = good.replace(&bad, "\"sessionId\":\"another\"");
        std::fs::write(
            &file,
            if bad.is_empty() {
                good.replace("\"sessionId\":", "\"x\":")
            } else {
                swapped
            },
        )
        .unwrap();
        let mut forged = input.clone();
        forged["transcript_path"] = path.clone();
        let out = post_raw(r.path(), &forged, NOW);
        assert!(
            String::from_utf8_lossy(&out.stderr).contains("not stored"),
            "{out:?}"
        );
        assert_eq!(stored(r.path()), json!({}));
    }
}

#[test]
fn a_transcript_without_the_tool_result_line_says_no_answer_yet() {
    let r = repo();
    let input = question(&[(12, &head("abc1234"))], "Hold");
    let mut open = input.clone();
    open["transcript_path"] = transcript(r.path(), &input, false);
    let out = post_raw(r.path(), &open, NOW);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("no answer yet") && err.contains("not stored"),
        "{err}"
    );
    assert_eq!(stored(r.path()), json!({}));
}
