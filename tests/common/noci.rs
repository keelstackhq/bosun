//! Verdicts that count without CI (spec §4): a gate App record, or a comment signed by an
//! enrolled key (the RFC 8032 test 1 key, `k1`).
#![allow(dead_code)]

use bosun::core::records::Record;
use bosun::core::verdict::{Verdict, comment_body, signed_message};
use bosun::github::b64;
use serde_json::json;

use super::TEST_ED25519_PEM;
use super::fake::FakeGithub;

const PUB: &str = "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=";

/// Enroll `k1` on the default branch.
pub fn enroll(fake: &FakeGithub) {
    enroll_role(fake, "reviewer");
}

/// Enroll `k1` with `role` (owner, reviewer, delegate or signer).
pub fn enroll_role(fake: &FakeGithub, role: &str) {
    let t = format!("[[key]]\nid = \"k1\"\ned25519 = \"{PUB}\"\nrole = \"{role}\"\n");
    fake.content(".bosun/approvers.toml", "refs/heads/main", &t);
}

fn pair() -> ring::signature::Ed25519KeyPair {
    let body: String = TEST_ED25519_PEM
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect();
    ring::signature::Ed25519KeyPair::from_pkcs8_maybe_unchecked(&b64::decode(&body).unwrap())
        .unwrap()
}

/// The verdict comment text of `session` for PR `pr` at `head`, unsigned; `decision` approve/reject.
pub fn verdict_text(pr: u64, head: &str, session: &str, decision: &str) -> String {
    let v = Verdict::parse(
        &json!({"pr": pr, "head_sha": head, "verdict": decision, "confidence": 0.95, "findings": []})
            .to_string(),
    )
    .unwrap();
    comment_body(&v, session, None)
}

/// `text` plus a signature line by `k1` for `repo` (`owner/name`), PR and head.
pub fn sign(text: &str, repo: &str, pr: u64, head: &str) -> String {
    let (o, n) = repo.split_once('/').unwrap();
    let msg = signed_message(o, n, pr, head, text);
    let sig = b64::encode(pair().sign(msg.as_bytes()).as_ref());
    format!("{text}\nbosun-sig ed25519 k1 {sig}")
}

/// Post a verdict comment by `user`, signed by `k1` for PR 20 at its current head.
pub fn signed_verdict(fake: &FakeGithub, user: &str, session: &str) {
    enroll(fake);
    let head = fake.pull_head(20);
    let text = verdict_text(20, &head, session, "approve");
    fake.comment_by(20, user, &sign(&text, "acme/widget", 20, &head));
}

/// Post the gate App's applied verdict record for `session` on PR 20's head.
pub fn app_verdict(fake: &FakeGithub, session: &str) {
    let head = fake.pull_head(20);
    let r = Record::Verdict {
        session: session.into(),
        head_sha: head,
        approved: true,
        confidence: Some(0.95),
        major: Some(0),
        model: None,
        relayed_by: None,
    };
    fake.comment_by(
        20,
        "github-actions[bot]",
        &format!("Review verdict: approve.\n\n{}", r.to_line()),
    );
}
