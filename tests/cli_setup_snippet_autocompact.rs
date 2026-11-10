//! `[session] autocompact_pct` and the env line `bosun setup-snippet` prints for it (spec §5).

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
fn a_set_pct_prints_the_export_line_and_where_it_goes() {
    let t = repo(Some("[session]\nautocompact_pct = 70\n"));
    let (code, v) = snippet(t.path());
    assert_eq!(code, 0, "{v}");
    let a = &v["data"]["autocompact"];
    assert_eq!(a["export"], "export CLAUDE_AUTOCOMPACT_PCT_OVERRIDE=70");
    assert_eq!(a["env"]["CLAUDE_AUTOCOMPACT_PCT_OVERRIDE"], "70");
    assert!(a["paste_into"].as_str().unwrap().contains("Environment"));
    assert!(
        a["alternative"]
            .as_str()
            .unwrap()
            .contains("CLAUDE_CODE_AUTO_COMPACT_WINDOW")
    );
    assert!(
        !v["data"]["script"]
            .as_str()
            .unwrap()
            .contains("AUTOCOMPACT"),
        "the bootstrap stays the same for every repo"
    );
}

#[test]
fn unset_or_no_config_prints_no_autocompact() {
    for toml in [None, Some("project = \"x\"\n")] {
        let t = repo(toml);
        let (code, v) = snippet(t.path());
        assert_eq!(code, 0, "{v}");
        assert!(v["data"].get("autocompact").is_none(), "{v}");
    }
}

#[test]
fn out_of_range_pct_is_refused_with_a_fix() {
    for bad in ["0", "101"] {
        let t = repo(Some(&format!("[session]\nautocompact_pct = {bad}\n")));
        let (code, v) = snippet(t.path());
        assert_eq!(code, 1, "{v}");
        let fix = v["error"]["fix"].as_str().unwrap();
        assert!(fix.contains("session.autocompact_pct"), "{fix}");
    }
}
