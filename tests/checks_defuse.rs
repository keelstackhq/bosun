//! The App's verdict record (spec §15, UC-B2-7) and the `[checks]` App guard.

use bosun::config::Config;
use bosun::github::gate::defuse;

#[test]
fn defuse_breaks_triggers_but_keeps_the_record_line_intact() {
    let line =
        r#"{"bosun":"verdict","session":"s/bosun recheck <!-- x","head_sha":"h","approved":true}"#;
    let body = format!("Review verdict: ok /bosun approve h <!-- bosun:verdict -->.\n\n{line}");
    let out = defuse(&body);
    for bad in ["<!--", "/bosun"] {
        assert!(!out.contains(bad), "{bad} in {out}");
    }
    let (human, record) = out.rsplit_once("\n\n").unwrap();
    assert!(human.starts_with("Review verdict: "), "{human}");
    let a: serde_json::Value = serde_json::from_str(record).unwrap();
    let b: serde_json::Value = serde_json::from_str(line).unwrap();
    assert_eq!(a, b, "the record reads back the same, session included");
}

#[test]
fn the_gate_app_is_never_github_actions() {
    for bad in [
        "app_id = 15368\napp_slug = \"gate\"\n",
        "app_id = 4242\napp_slug = \"github-actions\"\n",
        "app_id = 4242\napp_slug = \"GitHub-Actions\"\n",
    ] {
        let text = format!("[checks]\nmode = \"dual\"\n{bad}");
        let e = Config::parse(&text, ".bosun/bosun.toml").unwrap_err();
        assert!(e.message.contains("GitHub Actions"), "{bad}: {}", e.message);
        assert!(!e.fix.is_empty());
    }
}

#[test]
fn a_defused_record_still_counts_as_the_same_session() {
    use bosun::core::risk::apply_record;
    let line = r#"{"bosun":"verdict","session":"s/bosun x","head_sha":"h","approved":true,"confidence":0.95}"#;
    let body = format!("Review verdict: x.\n\n{line}");
    let before = apply_record(&body).expect("bosun's own shape");
    assert_eq!(apply_record(&defuse(&body)), Some(before));
}
