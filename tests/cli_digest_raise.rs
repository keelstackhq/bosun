//! `bosun digest --answer --sign-key`: budget raises (spec §14, plan PR 4b).
mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::{Value, json};

/// The RFC 8032 test 1 key, the public half of `common::TEST_ED25519_PEM`.
const PUB: &str = "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=";

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
    let v = one_object(&out.stdout);
    assert!(
        !v.to_string().contains("canary-owner-token"),
        "token leaked"
    );
    assert!(!v.to_string().contains("PRIVATE KEY"), "key leaked");
    if v["ok"] == false {
        assert!(
            !v["error"]["fix"].as_str().unwrap().trim().is_empty(),
            "{v}"
        );
    }
    (out.status.code().unwrap(), v)
}

fn enroll(fake: &FakeGithub, role: &str) {
    let keys = format!("[[key]]\nid = \"k1\"\ned25519 = \"{PUB}\"\nrole = \"{role}\"\n");
    fake.content(".bosun/approvers.toml", "refs/heads/main", &keys);
}

/// The digest header and id of an empty board.
fn header(fake: &FakeGithub) -> (String, String) {
    let (code, v) = digest(fake, &[], "");
    assert_eq!(code, 0, "{v}");
    let d = &v["data"];
    let s = |k: &str| d[k].as_str().unwrap().to_string();
    (s("header"), s("message_id"))
}

#[test]
fn a_raise_is_signed_only_with_an_enrolled_owner_key() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("k.pem");
    std::fs::write(&key, common::TEST_ED25519_PEM).unwrap();
    let key = key.to_str().unwrap();
    let fake = FakeGithub::start();
    let (head, id) = header(&fake);
    let reply = format!("{id} raise 7 +20");
    let input = format!("{head}\n{reply}\n");
    // Without --sign-key, with no approvers file, or with a delegate key: refused, exit 0.
    let (code, v) = digest(&fake, &["--answer"], &input);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["raises"], json!([]));
    assert_eq!(
        v["data"]["refused"][0]["raise"],
        json!({"epic": 7, "usd": 20})
    );
    let (code, v) = digest(&fake, &["--answer", "--sign-key", key], &input);
    assert_eq!(code, 0, "{v}");
    let why = v["data"]["refused"][0]["why"].as_str().unwrap();
    assert!(why.contains("approvers.toml"), "{why}");
    enroll(&fake, "delegate");
    let (_, v) = digest(&fake, &["--answer", "--sign-key", key], &input);
    assert_eq!(v["data"]["raises"], json!([]));
    assert!(
        v["data"]["refused"][0]["why"]
            .as_str()
            .unwrap()
            .contains("not an owner key")
    );
    // An owner key signs the raise, bound to repo, digest, day and the exact reply.
    enroll(&fake, "owner");
    let (code, v) = digest(&fake, &["--answer", "--sign-key", key], &input);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["refused"], json!([]));
    let r = &v["data"]["raises"][0];
    assert_eq!((r["epic"].as_u64(), r["usd"].as_u64()), (Some(7), Some(20)));
    assert_eq!(r["key_id"], json!("k1"));
    let sha = bosun::core::answer::text_sha256(&reply);
    let msg =
        format!("bosun-raise v1 acme/widget digest {id} day 2026-11-12 reply {sha} epic 7 usd +20");
    assert_eq!(r["message"], json!(msg));
    let sig = bosun::github::b64::decode(r["sig"].as_str().unwrap()).unwrap();
    let public = bosun::github::b64::decode(PUB).unwrap();
    assert!(bosun::core::approve::verify_ed25519(
        &public,
        msg.as_bytes(),
        &sig
    ));
    assert_eq!(v["next"], json!([]), "a raise is no approve call");
    // Another reply text signs another message.
    let (_, w) = digest(
        &fake,
        &["--answer", "--sign-key", key],
        &format!("{head}\n{id} raise 7 +20\nthanks\n"),
    );
    assert_ne!(w["data"]["raises"][0]["message"], r["message"]);
}

#[test]
fn bad_raises_are_unknown_and_sign_key_needs_answer() {
    let fake = FakeGithub::start();
    let (head, id) = header(&fake);
    let input = format!("{head}\n{id} raise 7 20\n{id} raise 7 +0\n{id} raise x +5\n");
    let (code, v) = digest(&fake, &["--answer"], &input);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["refused"], json!([]));
    assert_eq!(v["data"]["unknown"].as_array().unwrap().len(), 3);
    let (code, _) = digest(&fake, &["--sign-key", "k.pem"], "");
    assert_eq!(code, 2, "--sign-key needs --answer");
}
