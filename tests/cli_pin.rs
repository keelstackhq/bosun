mod common;

use common::{bosun, one_object};
use serde_json::{Value, json};
use std::path::Path;

use common::npm::{PINS, Registry, repo, sha};

fn read(d: &Path, rel: &str) -> String {
    std::fs::read_to_string(d.join(rel)).unwrap()
}

fn snapshot(d: &Path) -> Vec<String> {
    [
        "docs/setup-script.sh",
        "channel/pieces/core/files/docs/setup-script.sh.jinja",
        ".bosun/version",
    ]
    .iter()
    .map(|r| read(d, r))
    .collect()
}

fn pin(d: &Path, reg: &Registry, extra: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env("BOSUN_NPM_REGISTRY", &reg.base)
        .args(["-C", d.to_str().unwrap(), "pin", "--latest"])
        .args(extra)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

fn same_as_pins() -> Vec<(&'static str, &'static str)> {
    PINS.to_vec()
}

#[test]
fn bump_rewrites_the_script_the_version_file_and_the_template() {
    let d = repo();
    let reg = Registry::start(
        &[
            ("splice", "0.4.0"),
            ("hoist", "0.4.1"),
            ("berth", "0.1.0"),
            ("bosun", "0.9.1"),
        ],
        None,
        None,
    );
    let (code, v) = pin(d.path(), &reg, &[]);
    assert_eq!(code, 0, "{v}");
    let data = &v["data"];
    assert_eq!(
        data["changed"],
        json!([{"tool": "hoist", "from": "0.4.0", "to": "0.4.1"}, {"tool": "bosun", "from": "0.9.0", "to": "0.9.1"}])
    );
    assert_eq!(data["unchanged"], json!(["splice", "berth"]));
    assert_eq!(data["branch"], "bosun/pin");
    assert_eq!(data["title"], "chore: pin hoist 0.4.1, bosun 0.9.1");
    let script = read(d.path(), "docs/setup-script.sh");
    let tar = |p: &str, ver: &str| sha(format!("{p}@{ver}").as_bytes());
    for (suffix, p) in [
        ("", "hoist"),
        ("LINUX_X64_", "hoist-linux-x64"),
        ("LINUX_ARM64_", "hoist-linux-arm64"),
    ] {
        assert!(
            script.contains(&format!("HOIST_{suffix}SHA512={}\n", tar(p, "0.4.1"))),
            "{suffix}"
        );
    }
    assert!(script.contains("HOIST_VERSION=0.4.1\n") && script.contains("BOSUN_VERSION=0.9.1\n"));
    assert!(script.contains("SPLICE_VERSION=0.4.0\nSPLICE_SHA512=old\n"));
    assert_eq!(read(d.path(), ".bosun/version"), "0.9.1\n");
    let tpl = read(
        d.path(),
        "channel/pieces/core/files/docs/setup-script.sh.jinja",
    );
    assert!(tpl.contains("HOIST_VERSION=0.4.1\n") && tpl.contains("SPLICE_VERSION=0.4.0\n"));
    assert!(tpl.contains("BOSUN_VERSION={% if bosun_version %}x{% endif %}\n"));
    let files = data["files"].as_array().unwrap();
    assert_eq!(files.len(), 3, "{files:?}");
}

#[test]
fn equal_versions_change_nothing() {
    let d = repo();
    let before = snapshot(d.path());
    let reg = Registry::start(&same_as_pins(), None, None);
    let (code, v) = pin(d.path(), &reg, &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["changed"], json!([]));
    assert_eq!(
        v["data"]["unchanged"],
        json!(["splice", "hoist", "berth", "bosun"])
    );
    assert_eq!(snapshot(d.path()), before);
    assert_eq!(reg.tarballs(), 0);
}

#[test]
fn a_downgrade_is_a_conflict_and_writes_nothing() {
    let d = repo();
    let before = snapshot(d.path());
    let reg = Registry::start(
        &[
            ("splice", "0.5.0"),
            ("hoist", "0.3.9"),
            ("berth", "0.1.0"),
            ("bosun", "0.9.0"),
        ],
        None,
        None,
    );
    let (code, v) = pin(d.path(), &reg, &[]);
    assert_eq!(code, 3, "{v}");
    assert!(v["error"]["message"].as_str().unwrap().contains("hoist"));
    assert_eq!(snapshot(d.path()), before);
}

#[test]
fn a_hash_mismatch_is_refused_and_writes_nothing() {
    let d = repo();
    let before = snapshot(d.path());
    let reg = Registry::start(
        &[
            ("splice", "0.4.1"),
            ("hoist", "0.4.1"),
            ("berth", "0.1.0"),
            ("bosun", "0.9.0"),
        ],
        Some("hoist-linux-arm64"),
        None,
    );
    let (code, v) = pin(d.path(), &reg, &[]);
    assert_eq!(code, 1, "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("hoist-linux-arm64") && msg.contains("0.4.1"),
        "{msg}"
    );
    assert_eq!(snapshot(d.path()), before);
}

#[test]
fn a_tarball_outside_the_registry_is_refused() {
    let d = repo();
    let before = snapshot(d.path());
    let reg = Registry::start(
        &[
            ("splice", "0.4.0"),
            ("hoist", "0.4.1"),
            ("berth", "0.1.0"),
            ("bosun", "0.9.0"),
        ],
        None,
        Some("hoist-linux-x64"),
    );
    let (code, v) = pin(d.path(), &reg, &[]);
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("hoist-linux-x64")
    );
    assert_eq!(snapshot(d.path()), before);
}

#[test]
fn a_channel_consumer_is_sent_to_splice_update() {
    let d = repo();
    std::fs::create_dir_all(d.path().join(".splice/channels")).unwrap();
    std::fs::write(d.path().join(".splice/channels/bosun.toml"), "x = 1\n").unwrap();
    let before = snapshot(d.path());
    let reg = Registry::start(&[("hoist", "0.4.1")], None, None);
    let (code, v) = pin(d.path(), &reg, &[]);
    assert_eq!(code, 1, "{v}");
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("splice update --channel bosun")
    );
    assert_eq!(snapshot(d.path()), before);
    assert!(reg.log.lock().unwrap().is_empty());
}

#[test]
fn check_lists_what_is_behind_without_tarballs_or_writes() {
    let d = repo();
    let before = snapshot(d.path());
    let reg = Registry::start(
        &[
            ("splice", "0.4.0"),
            ("hoist", "0.4.1"),
            ("berth", "0.1.0"),
            ("bosun", "0.9.0"),
        ],
        None,
        None,
    );
    let (code, v) = pin(d.path(), &reg, &["--check"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        v["data"]["behind"],
        json!([{"tool": "hoist", "from": "0.4.0", "to": "0.4.1"}])
    );
    assert_eq!(snapshot(d.path()), before);
    assert_eq!(reg.tarballs(), 0);
}
