mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;
use std::path::Path;

fn step6(dir: &Path) -> Value {
    let fake = FakeGithub::start();
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", "2026-11-03T10:00:00Z")
        .arg("-C")
        .arg(dir)
        .arg("doctor")
        .output()
        .unwrap();
    one_object(&out.stdout)["data"]["items"][5].clone()
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

#[test]
fn step_6_prints_the_setup_script_and_names_the_environment() {
    let tmp = tempfile::tempdir().unwrap();
    let script = "#!/bin/sh\nset -e\necho hi\n";
    write(tmp.path(), "docs/setup-script.sh", script);
    write(tmp.path(), ".bosun/version", "0.4.0\n");
    let it = step6(tmp.path());
    assert_eq!(it["step"], 6);
    assert_eq!(it["status"], "ok", "{it}");
    assert_eq!(it["content"], script);
    assert_eq!(it["environment"], "keelstack");
    assert!(it["fix"].as_str().unwrap().contains("keelstack"));
    assert!(it["detail"].as_str().unwrap().contains("keelstack"));
}

#[test]
fn step_6_without_the_script_is_missing_with_null_content() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), ".bosun/version", "0.4.0\n");
    let it = step6(tmp.path());
    assert_eq!(it["status"], "missing");
    assert!(it["content"].is_null());
    assert_eq!(it["environment"], "keelstack");
}

fn problem(it: &Value) -> String {
    assert_eq!(it["status"], "missing", "{it}");
    assert!(it["content"].is_null(), "{it}");
    it["detail"].as_str().unwrap().to_string()
}

#[cfg(unix)]
#[test]
fn step_6_refuses_a_symlinked_script() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "real.sh", "#!/bin/sh\n");
    write(tmp.path(), ".bosun/version", "0.4.0\n");
    std::fs::create_dir_all(tmp.path().join("docs")).unwrap();
    std::os::unix::fs::symlink(
        tmp.path().join("real.sh"),
        tmp.path().join("docs/setup-script.sh"),
    )
    .unwrap();
    assert!(problem(&step6(tmp.path())).contains("is a symlink"));
}

#[test]
fn step_6_reports_a_non_utf8_script_as_unreadable() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), ".bosun/version", "0.4.0\n");
    std::fs::create_dir_all(tmp.path().join("docs")).unwrap();
    std::fs::write(tmp.path().join("docs/setup-script.sh"), [0xff, 0xfe, 0x00]).unwrap();
    let d = problem(&step6(tmp.path()));
    assert!(d.contains("is not readable"), "{d}");
    assert!(!d.contains("is missing"));
}

#[test]
fn step_6_caps_the_script_at_64_kib() {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), ".bosun/version", "0.4.0\n");
    write(
        tmp.path(),
        "docs/setup-script.sh",
        &"#".repeat(64 * 1024 + 1),
    );
    assert!(problem(&step6(tmp.path())).contains("64 KiB"));
    write(tmp.path(), "docs/setup-script.sh", &"#".repeat(64 * 1024));
    assert_eq!(step6(tmp.path())["status"], "ok");
}
