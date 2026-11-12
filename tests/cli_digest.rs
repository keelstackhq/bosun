mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

const WAITING: &str = "high risk, owner approval needed: R-gov, R-ci";
/// The RFC 8032 test 1 key, the public half of `common::TEST_ED25519_PEM`.
const PUB: &str = "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=";

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
    fake
}

fn digest(fake: &FakeGithub, extra: &[&str], input: &str) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_OWNER_TOKEN", "canary-owner-token-7f3a")
        .env("BOSUN_HTTP_MAX_SLEEP_MS", "0")
        .env("BOSUN_SESSION", "session_orch")
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
        "Daily digest {id}\nApprovals (2):\n- #12@abc1234 R-gov, R-ci: `first change`\n- #15@def5678 R-gov, R-ci: `second change`\nOpen asks (1):\n- #30 `which db?`\nBudget asks (0):\nReply: approve 12 15 / raise <epic> +<usd>"
    );
    assert_eq!(d["message"], json!(want));
    let pairs = format!("12@{} 15@{}", head("abc1234"), head("def5678"));
    assert_eq!(d["header"], json!(format!("/bosun digest {id} {pairs}")));
    assert_eq!(
        d["bypass"],
        json!([{"pr": 20, "why": "mainfix: approve it now, not in the digest"}])
    );
    assert_eq!(
        d["asks"],
        json!([{"issue": 30, "answer": "bosun answer 30 --chat"}])
    );
    assert_eq!(writes(&fake), 0, "the digest writes nothing");
}

#[test]
fn the_answer_returns_approve_calls_and_drops_a_moved_head() {
    let fake = setup();
    let (_, v) = digest(&fake, &[], "");
    let header = v["data"]["header"].as_str().unwrap().to_string();
    let moved = head("def9999");
    set_head(&fake, 15, &moved);
    let input = format!("{header}\nApprove 12, 15, 20\nraise 7 +20\nthanks\n");
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
    assert_eq!(d["raises"], json!([]));
    assert_eq!(d["refused"][0]["raise"], json!({"epic": 7, "usd": 20}));
    assert_eq!(d["unknown"], json!(["thanks"]));
    assert_eq!(writes(&fake), 0, "the answer writes nothing");
}

#[test]
fn a_raise_counts_only_signed_by_an_enrolled_owner_key() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("k.pem");
    std::fs::write(&key, common::TEST_ED25519_PEM).unwrap();
    let key = key.to_str().unwrap();
    let fake = setup();
    let (_, v) = digest(&fake, &[], "");
    let id = v["data"]["message_id"].as_str().unwrap().to_string();
    let input = format!("{}\nraise 7 +20\n", v["data"]["header"].as_str().unwrap());
    // No approvers file: nothing is signed.
    let (code, v) = digest(&fake, &["--answer", "--sign-key", key], &input);
    assert_eq!(code, 1, "{v}");
    for (role, ok) in [("delegate", false), ("owner", true)] {
        let keys = format!("[[key]]\nid = \"k1\"\ned25519 = \"{PUB}\"\nrole = \"{role}\"\n");
        fake.content(".bosun/approvers.toml", "refs/heads/main", &keys);
        let (code, v) = digest(&fake, &["--answer", "--sign-key", key], &input);
        if !ok {
            assert_eq!(code, 1, "{v}");
            continue;
        }
        assert_eq!(code, 0, "{v}");
        let r = &v["data"]["raises"][0];
        assert_eq!((r["epic"].as_u64(), r["usd"].as_u64()), (Some(7), Some(20)));
        assert_eq!(r["key_id"], json!("k1"));
        let msg = format!("bosun-raise v1 acme/widget digest {id} epic 7 usd +20");
        assert_eq!(r["message"], json!(msg));
        let sig = bosun::github::b64::decode(r["sig"].as_str().unwrap()).unwrap();
        let public = bosun::github::b64::decode(PUB).unwrap();
        assert!(bosun::core::approve::verify_ed25519(
            &public,
            msg.as_bytes(),
            &sig
        ));
        assert!(!v.to_string().contains("PRIVATE KEY"));
    }
}

#[test]
fn an_answer_without_a_valid_header_is_a_usage_error() {
    let fake = setup();
    for input in [
        "approve 12\n",
        "/bosun digest 00000000 12@abc\napprove 12\n",
    ] {
        let (code, v) = digest(&fake, &["--answer"], input);
        assert_eq!(code, 2, "{v}");
    }
    let (code, _) = digest(&FakeGithub::start(), &["--sign-key", "k.pem"], "");
    assert_eq!(code, 2, "--sign-key needs --answer");
}
