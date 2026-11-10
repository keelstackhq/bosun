//! `[session] autocompact_window` / `autocompact_pct` and the env lines `bosun setup-snippet`
//! prints for them (spec §5). Cloud sessions set the pct variable themselves, so only the window
//! goes into a cloud environment; pct is for local sessions.

mod common;

use std::path::Path;
use std::process::Command;

use common::{bosun, one_object};
use serde_json::Value;

fn repo(toml: Option<&str>) -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    let ok = Command::new("git")
        .args(["init", "-q"])
        .current_dir(t.path())
        .status()
        .unwrap();
    assert!(ok.success());
    if let Some(text) = toml {
        std::fs::create_dir_all(t.path().join(".bosun")).unwrap();
        std::fs::write(t.path().join(".bosun/bosun.toml"), text).unwrap();
    }
    t
}

fn snippet(dir: &Path) -> (i32, Value) {
    let out = bosun()
        .current_dir(dir)
        .arg("setup-snippet")
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

#[test]
fn a_window_goes_into_the_cloud_environment() {
    let t = repo(Some("[session]\nautocompact_window = 200000\n"));
    let (code, v) = snippet(t.path());
    assert_eq!(code, 0, "{v}");
    let a = &v["data"]["autocompact"];
    assert_eq!(a["cloud_env"]["CLAUDE_CODE_AUTO_COMPACT_WINDOW"], "200000");
    assert_eq!(
        a["local_exports"][0],
        "export CLAUDE_CODE_AUTO_COMPACT_WINDOW=200000"
    );
    assert!(
        a["paste_into"]
            .as_str()
            .unwrap()
            .contains("Environment variables")
    );
    assert!(
        !v["data"]["script"]
            .as_str()
            .unwrap()
            .contains("AUTO_COMPACT"),
        "the bootstrap stays the same for every repo"
    );
}

#[test]
fn a_pct_is_local_only_and_never_a_cloud_variable() {
    let t = repo(Some("[session]\nautocompact_pct = 70\n"));
    let (code, v) = snippet(t.path());
    assert_eq!(code, 0, "{v}");
    let a = &v["data"]["autocompact"];
    assert_eq!(
        a["local_exports"][0],
        "export CLAUDE_AUTOCOMPACT_PCT_OVERRIDE=70"
    );
    assert_eq!(a["cloud_env"], serde_json::json!({}));
    assert!(a["note"].as_str().unwrap().contains("cloud"), "{a}");
}

#[test]
fn unset_no_config_or_another_bad_section_prints_no_autocompact() {
    for toml in [
        None,
        Some("project = \"x\"\n"),
        Some("[future_section]\nkey = 1\n"),
        Some("not = [valid\n"),
    ] {
        let t = repo(toml);
        let (code, v) = snippet(t.path());
        assert_eq!(code, 0, "{toml:?}: {v}");
        assert!(v["data"].get("autocompact").is_none(), "{v}");
    }
}

#[test]
fn bad_values_are_refused_naming_the_key() {
    for (key, bad) in [
        ("autocompact_pct", "0"),
        ("autocompact_pct", "101"),
        ("autocompact_pct", "\"70\""),
        ("autocompact_window", "99999"),
        ("autocompact_window", "1000001"),
    ] {
        let t = repo(Some(&format!("[session]\n{key} = {bad}\n")));
        let (code, v) = snippet(t.path());
        assert_eq!(code, 1, "{v}");
        let msg = v["error"]["message"].as_str().unwrap();
        assert!(msg.contains(&format!("session.{key}")), "{msg}");
        assert!(!msg.contains("expected"), "named, not serde: {msg}");
        let fix = v["error"]["fix"].as_str().unwrap();
        assert!(fix.contains(&format!("session.{key}")), "{fix}");
    }
}
