//! `bosun intake check|sign|verify` (the intake file, spec section 3) and the old
//! `bosun intake <text>` form next to them.
mod common;

use common::fake::FakeGithub;
use serde_json::Value;
use std::path::{Path, PathBuf};

const NOW: &str = "2026-11-02T10:00:00Z";
const PUB: &str = "11qYAYKxCrfVS/7TyWQHOg7hcvPapiMlrwIaaPcHURo=";
/// Never the signer: the RFC 8032 test 2 key.
const PUB2: &str = "PUAXw+hDiVqStwqnTRt+vJyYLM8uxJaMwM1V8Sr0Zgw=";

const OK: &str = r#"version = 1
repo = { name = "app", owner = "keelstackhq", visibility = "private" }
stack = ["rust"]
pieces = ["base", "rust-cli"]
bosun_pieces = ["claude-md"]
goal = "A CLI that does a thing"
publish = ["crates"]
approvals = "orchestrator"
notify = "chat"
slack_channel = ""
"#;

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    /// A working tree whose approvers file enrols key `orch` (the signer) with `role`.
    fn new(role: &str) -> Env {
        let e = Env {
            dir: tempfile::tempdir().unwrap(),
        };
        e.write("key.pem", common::TEST_ED25519_PEM);
        e.write(".bosun/intake.toml", OK);
        e.write(
            ".bosun/approvers.toml",
            &format!(
                "[[key]]\nid = \"orch\"\ned25519 = \"{PUB}\"\nrole = \"{role}\"\n\n[[key]]\nid = \"other\"\ned25519 = \"{PUB2}\"\n"
            ),
        );
        e
    }
    fn p(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }
    fn write(&self, rel: &str, text: &str) {
        let p = self.p(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    fn s(&self, rel: &str) -> String {
        self.p(rel).to_str().unwrap().to_string()
    }
    /// Run `bosun intake <args>` in the dir at `now`; asserts one object and a fix on errors.
    fn run(&self, now: &str, args: &[&str], stdin: &str) -> (i32, Value) {
        let out = common::bosun()
            .current_dir(self.dir.path())
            .env("BOSUN_NOW", now)
            .arg("intake")
            .args(args)
            .write_stdin(stdin.to_string())
            .output()
            .unwrap();
        let all = String::from_utf8_lossy(&out.stdout).to_string()
            + &String::from_utf8_lossy(&out.stderr);
        assert!(
            !all.contains("PRIVATE KEY") && !all.contains("MC4CAQAw"),
            "key leaked"
        );
        let v = common::one_object(&out.stdout);
        if v["ok"] == false {
            assert!(
                !v["error"]["fix"].as_str().unwrap().trim().is_empty(),
                "{v}"
            );
        }
        (out.status.code().unwrap(), v)
    }
    /// Sign `.bosun/intake.toml`; on success the printed signature goes to `.bosun/intake.sig`.
    fn sign(&self, extra: &[&str]) -> (i32, Value) {
        let key = self.s("key.pem");
        let mut a = vec!["sign", ".bosun/intake.toml", "--sign-key", &key];
        a.extend_from_slice(extra);
        let (code, v) = self.run(NOW, &a, "");
        if code == 0 {
            self.write(".bosun/intake.sig", v["data"]["sig"].as_str().unwrap());
        }
        (code, v)
    }
    fn verify(&self, now: &str) -> (i32, Value) {
        let a = [
            "verify",
            ".bosun/intake.toml",
            "--sig",
            ".bosun/intake.sig",
            "--approvers",
            ".bosun/approvers.toml",
        ];
        self.run(now, &a, "")
    }
}

const SIGN: [&str; 4] = ["--repo", "KeelstackHQ/App", "--mode", "new"];

fn sign_args<'a>(more: &[&'a str]) -> Vec<&'a str> {
    [SIGN.as_slice(), more].concat()
}

#[test]
fn check_validates_the_schema_and_reports_the_fields() {
    let e = Env::new("owner");
    let (code, v) = e.run(NOW, &["check"], "");
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "intake check");
    assert_eq!(v["data"]["repo"]["name"], "app");
    assert_eq!(v["data"]["stack"][0], "rust");
    let (code, v) = e.run(NOW, &["check", "--from", "-"], OK);
    assert_eq!(code, 0, "{v}");
    let (code, v) = e.run(NOW, &["check", "nope.toml"], "");
    assert_eq!(code, 2, "{v}");
}

#[test]
fn check_separates_a_missing_answer_from_a_bad_value() {
    let e = Env::new("owner");
    let cases = [
        ("stack = [\"rust\"]\n", "", 5, "stack"),
        (
            "goal = \"A CLI that does a thing\"",
            "goal = \"\"",
            5,
            "goal",
        ),
        ("\"private\"", "\"secret\"", 2, "repo.visibility"),
        ("[\"crates\"]", "[\"crates\", \"none\"]", 2, "publish"),
        ("\"chat\"", "\"slack\"", 5, "slack_channel"),
        ("goal = ", "plan = \"plans/a.md\"\ngoal = ", 2, "goal"),
        (
            "version = 1\n",
            "version = 1\nsurprise = 1\n",
            2,
            "surprise",
        ),
        (
            "visibility = \"private\"",
            "visibility = \"private\", x = 1",
            2,
            "repo.x",
        ),
        ("stack = [\"rust\"]", "stack = \"rust\"", 2, "stack"),
    ];
    for (from, to, want, field) in cases {
        let (code, v) = e.run(NOW, &["check", "--from", "-"], &OK.replacen(from, to, 1));
        assert_eq!(code, want, "{field}: {v}");
        let err = &v["error"];
        let text = format!("{} {}", err["message"], err["fix"]);
        assert!(text.contains(field), "{field}: {v}");
        assert_eq!(v["data"]["field"], field);
    }
    let (code, v) = e.run(NOW, &["check", "--from", "-"], "version = [");
    assert_eq!(code, 2, "{v}");
}

#[test]
fn check_ignores_signatures_and_a_signature_table_is_an_unknown_key() {
    let e = Env::new("owner");
    e.write(".bosun/intake.sig", "garbage");
    let (code, v) = e.run(NOW, &["check"], "");
    assert_eq!(code, 0, "{v}");
    assert!(v["data"].get("signed").is_none());
    let with_table = format!("{OK}\n[signature]\nkey = \"k\"\nsig = \"s\"\n");
    let (code, v) = e.run(NOW, &["check", "--from", "-"], &with_table);
    assert_eq!(code, 2, "{v}");
}

#[test]
fn a_signed_intake_round_trips_through_sign_and_verify() {
    let e = Env::new("owner");
    let (code, v) = e.sign(&sign_args(&["--key-id", "orch"]));
    assert_eq!(code, 0, "{v}");
    let sig = v["data"]["sig"].as_str().unwrap();
    assert!(sig.contains("key = \"orch\"") && sig.contains("mode = \"new\""));
    assert!(sig.contains("expires = \"2026-11-02T11:00:00Z\""));
    let (code, v) = e.verify(NOW);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["key"], "orch");
    assert_eq!(v["data"]["mode"], "new");
    assert_eq!(v["data"]["repo"], "keelstackhq/app");
    assert_eq!(v["data"]["nonce"].as_str().unwrap().len(), 32);
}

#[test]
fn sign_writes_nothing_and_finds_the_key_id_in_approvers() {
    let e = Env::new("owner");
    let key = e.s("key.pem");
    let a = [
        "sign",
        ".bosun/intake.toml",
        "--sign-key",
        &key,
        "--repo",
        "keelstackhq/app",
        "--mode",
        "adopt",
    ];
    let (code, v) = e.run(NOW, &a, "");
    assert_eq!(code, 0, "{v}");
    assert!(
        v["data"]["sig"]
            .as_str()
            .unwrap()
            .contains("key = \"orch\"")
    );
    assert!(!e.p(".bosun/intake.sig").exists());
    // Two signatures differ by their nonce.
    let (_, w) = e.run(NOW, &a, "");
    assert_ne!(v["data"]["sig"], w["data"]["sig"]);
    // A key that is not enrolled, and no --key-id: refused.
    e.write(".bosun/approvers.toml", "");
    let (code, v) = e.run(NOW, &a, "");
    assert_eq!(code, 1, "{v}");
}

#[test]
fn sign_refuses_a_wrong_repo_a_long_expiry_and_a_bad_intake() {
    let e = Env::new("owner");
    let id = ["--key-id", "orch"];
    let wrong = ["--repo", "keelstackhq/other", "--mode", "new"];
    let (code, v) = e.sign(&[wrong.as_slice(), &id].concat());
    assert_eq!(code, 2, "{v}");
    for secs in ["3601", "0"] {
        let (code, v) = e.sign(&sign_args(&["--key-id", "orch", "--expires-in", secs]));
        assert_eq!(code, 2, "{secs}: {v}");
    }
    let (code, v) = e.sign(&sign_args(&["--key-id", "orch", "--expires-in", "600"]));
    assert_eq!(code, 0, "{v}");
    assert!(
        v["data"]["sig"]
            .as_str()
            .unwrap()
            .contains("2026-11-02T10:10:00Z")
    );
    let (code, v) = e.run(
        NOW,
        &[
            "sign",
            "nope.toml",
            "--sign-key",
            &e.s("key.pem"),
            "--repo",
            "a/b",
            "--mode",
            "new",
        ],
        "",
    );
    assert_eq!(code, 2, "{v}");
    e.write(
        ".bosun/intake.toml",
        &OK.replace("stack = [\"rust\"]\n", ""),
    );
    let (code, v) = e.sign(&sign_args(&["--key-id", "orch"]));
    assert_eq!(code, 5, "{v}");
}

fn reason(v: &Value) -> &str {
    v["data"]["reason"].as_str().unwrap_or("")
}

#[test]
fn verify_fails_closed_with_a_named_reason() {
    let e = Env::new("owner");
    e.sign(&sign_args(&["--key-id", "orch"]));
    let sig = std::fs::read_to_string(e.p(".bosun/intake.sig")).unwrap();

    // A tampered intake byte.
    e.write(".bosun/intake.toml", &OK.replace("A CLI", "A CLJ"));
    let (code, v) = e.verify(NOW);
    assert_eq!((code, reason(&v)), (5, "intake_changed"), "{v}");
    e.write(".bosun/intake.toml", OK);

    // Mode is part of the signed message.
    e.write(
        ".bosun/intake.sig",
        &sig.replace("mode = \"new\"", "mode = \"adopt\""),
    );
    let (code, v) = e.verify(NOW);
    assert_eq!((code, reason(&v)), (5, "bad_signature"), "{v}");

    // A signature for another repo is not for this intake.
    e.write(
        ".bosun/intake.sig",
        &sig.replace("keelstackhq/app", "keelstackhq/other"),
    );
    let (code, v) = e.verify(NOW);
    assert_eq!((code, reason(&v)), (5, "repo_mismatch"), "{v}");

    // Expired: the instant of expiry already counts as expired.
    e.write(".bosun/intake.sig", &sig);
    let (code, v) = e.verify("2026-11-02T11:00:00Z");
    assert_eq!((code, reason(&v)), (5, "expired"), "{v}");
    let (code, _) = e.verify("2026-11-02T10:59:59Z");
    assert_eq!(code, 0);

    // A tampered signature.
    e.write(
        ".bosun/intake.sig",
        &sig.replace("sig = \"", "sig = \"AAAA"),
    );
    let (code, v) = e.verify(NOW);
    assert_eq!((code, reason(&v)), (5, "bad_signature"), "{v}");

    // An unreadable signature file.
    e.write(".bosun/intake.sig", "nonsense");
    let (code, v) = e.verify(NOW);
    assert_eq!((code, reason(&v)), (5, "bad_sig_file"), "{v}");
}

#[test]
fn verify_refuses_an_unknown_key_and_a_role_that_may_not_sign() {
    let e = Env::new("owner");
    e.sign(&sign_args(&["--key-id", "ghost"]));
    let (code, v) = e.verify(NOW);
    assert_eq!((code, reason(&v)), (5, "unknown_key"), "{v}");

    let e = Env::new("delegate");
    e.sign(&sign_args(&["--key-id", "orch"]));
    let (code, v) = e.verify(NOW);
    assert_eq!((code, reason(&v)), (5, "role_not_allowed"), "{v}");

    // Signed under the id of a key with another public key: the signature does not verify.
    let e = Env::new("owner");
    e.sign(&sign_args(&["--key-id", "other"]));
    let (code, v) = e.verify(NOW);
    assert_eq!((code, reason(&v)), (5, "bad_signature"), "{v}");
}

#[test]
fn verify_refuses_an_unusable_intake_or_approvers_file_as_exit_5() {
    let e = Env::new("owner");
    e.sign(&sign_args(&["--key-id", "orch"]));
    let ok = std::fs::read_to_string(e.p(".bosun/intake.toml")).unwrap();
    e.write(
        ".bosun/intake.toml",
        &ok.replace("\"private\"", "\"secret\""),
    );
    let (code, v) = e.verify(NOW);
    assert_eq!((code, reason(&v)), (5, "bad_intake"), "{v}");
    std::fs::remove_file(e.p(".bosun/intake.toml")).unwrap();
    let (code, v) = e.verify(NOW);
    assert_eq!((code, reason(&v)), (5, "bad_intake"), "{v}");
    e.write(".bosun/intake.toml", &ok);
    e.write(".bosun/approvers.toml", "[[key]]\nid = \"x\"\nbogus = 1\n");
    let (code, v) = e.verify(NOW);
    assert_eq!((code, reason(&v)), (5, "bad_approvers"), "{v}");
    std::fs::remove_file(e.p(".bosun/approvers.toml")).unwrap();
    let (code, v) = e.verify(NOW);
    assert_eq!((code, reason(&v)), (5, "bad_approvers"), "{v}");
}

#[test]
fn verify_refuses_an_expiry_more_than_an_hour_away() {
    let e = Env::new("owner");
    let key = e.s("key.pem");
    // Signing takes its clock from BOSUN_NOW, so a signer one second ahead of the verifier
    // makes a signature that expires 3601 s after the verifier's now.
    let ahead = "2026-11-02T10:00:01Z";
    let mut a = vec!["sign", ".bosun/intake.toml", "--sign-key", &key];
    a.extend(sign_args(&["--key-id", "orch"]));
    let (code, v) = e.run(ahead, &a, "");
    assert_eq!(code, 0, "{v}");
    e.write(".bosun/intake.sig", v["data"]["sig"].as_str().unwrap());
    let (code, v) = e.verify(NOW);
    assert_eq!((code, reason(&v)), (5, "expiry_too_far"), "{v}");
    // Exactly an hour is still fine.
    let (code, v) = e.verify(ahead);
    assert_eq!(code, 0, "{v}");
}

/// A signature file for `.bosun/intake.toml` that `sign` could not make: `issued` and `expires`
/// are chosen freely, signed with the test key.
fn forged_sig(e: &Env, issued: &str, expires: &str) -> String {
    use bosun::core::intake::{SigFile, intake_sha256};
    let text = std::fs::read_to_string(e.p(".bosun/intake.toml")).unwrap();
    let body: String = common::TEST_ED25519_PEM
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect();
    let der = bosun::github::b64::decode(&body).unwrap();
    let pair = ring::signature::Ed25519KeyPair::from_pkcs8_maybe_unchecked(&der).unwrap();
    let mut f = SigFile {
        key: "orch".into(),
        repo: "keelstackhq/app".into(),
        mode: "new".into(),
        nonce: "ab".repeat(16),
        issued: issued.into(),
        expires: expires.into(),
        intake_sha256: intake_sha256(text.as_bytes()),
        sig: String::new(),
    };
    f.sig = bosun::github::b64::encode(pair.sign(f.message().as_bytes()).as_ref());
    f.to_toml()
}

#[test]
fn verify_refuses_a_window_longer_than_an_hour() {
    let e = Env::new("owner");
    // Valid now and not too far ahead, but the window is 3601 s.
    e.write(
        ".bosun/intake.sig",
        &forged_sig(&e, "2026-11-02T09:10:00Z", "2026-11-02T10:10:01Z"),
    );
    let (code, v) = e.verify(NOW);
    assert_eq!((code, reason(&v)), (5, "window_too_long"), "{v}");
    // The same window cut to an hour verifies.
    e.write(
        ".bosun/intake.sig",
        &forged_sig(&e, "2026-11-02T09:10:00Z", "2026-11-02T10:10:00Z"),
    );
    let (code, v) = e.verify(NOW);
    assert_eq!(code, 0, "{v}");
}

#[test]
fn verify_refuses_a_window_minted_for_later() {
    let e = Env::new("owner");
    let key = e.s("key.pem");
    // Signed two hours ahead: issued and expires are both in the future, and the window is fine.
    let later = "2026-11-02T12:00:00Z";
    let mut a = vec!["sign", ".bosun/intake.toml", "--sign-key", &key];
    a.extend(sign_args(&["--key-id", "orch"]));
    let (code, v) = e.run(later, &a, "");
    assert_eq!(code, 0, "{v}");
    e.write(".bosun/intake.sig", v["data"]["sig"].as_str().unwrap());
    let (code, v) = e.verify(NOW);
    assert_eq!((code, reason(&v)), (5, "issued_in_future"), "{v}");
    let (code, v) = e.verify(later);
    assert_eq!(code, 0, "{v}");
    // Clock skew of up to 60 s is tolerated, 61 s is not.
    e.write(
        ".bosun/intake.sig",
        &forged_sig(&e, "2026-11-02T10:01:00Z", "2026-11-02T10:30:00Z"),
    );
    let (code, v) = e.verify(NOW);
    assert_eq!(code, 0, "{v}");
    e.write(
        ".bosun/intake.sig",
        &forged_sig(&e, "2026-11-02T10:01:01Z", "2026-11-02T10:30:00Z"),
    );
    let (code, v) = e.verify(NOW);
    assert_eq!((code, reason(&v)), (5, "issued_in_future"), "{v}");
}

#[test]
fn the_sign_key_path_never_reaches_the_envelope() {
    let e = Env::new("owner");
    let secret = e.s("secret-key-path-xyz.pem");
    for content in [None, Some("not a pem")] {
        if let Some(c) = content {
            std::fs::write(&secret, c).unwrap();
        }
        let mut a = vec!["sign", ".bosun/intake.toml", "--sign-key", &secret];
        a.extend(sign_args(&["--key-id", "orch"]));
        let out = common::bosun()
            .current_dir(e.dir.path())
            .arg("intake")
            .args(&a)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        let v = common::one_object(&out.stdout);
        assert!(!v.to_string().contains("secret-key-path-xyz"), "{v}");
        assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    }
}

#[test]
fn check_refuses_a_file_together_with_from_stdin() {
    let e = Env::new("owner");
    let (code, v) = e.run(NOW, &["check", ".bosun/intake.toml", "--from", "-"], OK);
    assert_eq!(code, 2, "{v}");
    assert_eq!(v["command"], "intake check");
}

#[test]
fn a_missing_default_file_needs_the_owner_but_a_named_one_is_usage() {
    let e = Env::new("owner");
    std::fs::remove_file(e.p(".bosun/intake.toml")).unwrap();
    let (code, v) = e.run(NOW, &["check"], "");
    assert_eq!(code, 5, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("bosun init --intake")
    );
    let (code, v) = e.run(NOW, &["check", "other.toml"], "");
    assert_eq!(code, 2, "{v}");
}

#[test]
fn structural_errors_name_the_key() {
    let e = Env::new("owner");
    for (src, field) in [
        (format!("{OK}extra = 1\n"), "extra"),
        (OK.replace("repo = {", "repo = 5 #{"), "repo"),
    ] {
        let (code, v) = e.run(NOW, &["check", "--from", "-"], &src);
        assert_eq!(code, 2, "{v}");
        assert_eq!(v["data"]["field"], field, "{v}");
    }
}

// UC-ZI-7: the old form fails with exit 2 before it needs any repo context.
#[test]
fn the_old_form_checks_its_arguments_before_any_repo_or_github_access() {
    let d = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        let out = common::bosun()
            .current_dir(d.path())
            .env_remove("BOSUN_REPO")
            .env("BOSUN_HALT", "1")
            .arg("intake")
            .args(args)
            .output()
            .unwrap();
        (out.status.code().unwrap(), common::one_object(&out.stdout))
    };
    for args in [
        vec!["x"],
        vec!["", "--source", "human"],
        vec!["  ", "--source", "human"],
    ] {
        let (code, v) = run(&args);
        assert_eq!(code, 2, "{args:?}: {v}");
        assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    }
}

// UC-ZI-7: the old form keeps working.
#[test]
fn the_old_intake_text_form_still_opens_a_plan_issue() {
    let fake = FakeGithub::start();
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("plans")).unwrap();
    std::fs::write(
        d.path().join("plans/widget.md"),
        "# Plan: widget\n\nproject: widget\n\n## M1: Lifecycle\n\narea: lifecycle\n\n### Tasks\n\n- T1: State machine\n  touch: src/core/state.rs\n\n### Done when\n\n- UC-M1-1: a task moves planned -> merged.\n",
    )
    .unwrap();
    let dir: &Path = d.path();
    let args = [
        "-C",
        dir.to_str().unwrap(),
        "intake",
        "add a flag",
        "--source",
        "human",
    ];
    let (code, v) = common::run(&fake, "session_test", NOW, &args);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["command"], "intake");
    assert_eq!(v["data"]["created"], true);
    let issue = fake.issue_state(v["data"]["issue"].as_u64().unwrap());
    assert!(issue.labels.contains(&"bosun:source/human".to_string()));
    // The source is still required.
    let (code, v) = common::run(&fake, "session_test", NOW, &["intake", "x"]);
    assert_eq!(code, 2, "{v}");
}
