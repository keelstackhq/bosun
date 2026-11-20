//! `bosun setup-snippet --install-hooks`: updates in place, follows symlinks, writes privately.
mod common;

use common::{bosun, one_object};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn install(settings: &Path) -> (i32, Value) {
    let out = bosun()
        .args(["setup-snippet", "--install-hooks", "--settings"])
        .arg(settings)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

fn read(p: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn same_command_with_another_matcher_or_timeout_is_updated_not_duplicated() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("settings.json");
    let narrow = json!({"hooks": {
        "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "bosun hook pre-tool-use"}]}],
        "PreCompact": [{"hooks": [{"type": "command", "command": "out=$(bosun hook pre-compact) && printf '%s\\n' \"$out\"; exit 0", "timeout": 3}]}],
    }, "other": 1});
    fs::write(&f, narrow.to_string()).unwrap();
    let (code, v) = install(&f);
    assert_eq!(code, 0, "{v}");
    let mut updated: Vec<String> = serde_json::from_value(v["data"]["updated"].clone()).unwrap();
    updated.sort();
    assert_eq!(updated, ["PreCompact", "PreToolUse"], "{v}");
    let doc = read(&f);
    assert_eq!(doc["other"], 1);
    let pre = doc["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(pre.len(), 1, "{doc}");
    assert!(
        pre[0]["matcher"].as_str().unwrap().contains("NotebookEdit"),
        "{doc}"
    );
    assert_eq!(doc["hooks"]["PreCompact"].as_array().unwrap().len(), 1);
    assert_eq!(doc["hooks"]["PreCompact"][0]["hooks"][0]["timeout"], 15);
    // A second run changes nothing.
    let (_, v) = install(&f);
    assert_eq!(v["data"]["added"], json!([]));
    assert_eq!(v["data"]["updated"], json!([]));
}

#[test]
fn a_symlinked_settings_file_is_written_through() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.json");
    let link = dir.path().join("settings.json");
    fs::write(&real, "{}").unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o640)).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let (code, v) = install(&link);
    assert_eq!(code, 0, "{v}");
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(read(&real)["hooks"]["SessionStart"].is_array());
    let mode = fs::metadata(&real).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o640);
}

#[test]
fn a_dangling_symlink_is_an_error_and_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let link = dir.path().join("settings.json");
    std::os::unix::fs::symlink(dir.path().join("missing.json"), &link).unwrap();
    let (code, v) = install(&link);
    assert_eq!(code, 1, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!dir.path().join("missing.json").exists());
}

#[test]
fn a_new_settings_file_is_private() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("sub/settings.json");
    let (code, v) = install(&f);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        fs::metadata(&f).unwrap().permissions().mode() & 0o777,
        0o600
    );
}
