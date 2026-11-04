mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;
use std::path::Path;

const M: &str = "5c9072877417873fee42be1805aa776a453ec394";
const P: &str = "22f0b463e7a5e579904dd6211a520321035526f7";
const C: &str = "a9cc1f7b82598852e8c6b8e599dde6ac2491b26e";

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// Doctor's step 13; `config` is CLAUDE_CONFIG_DIR, or None to use HOME.
fn step13(repo: &Path, config: Option<&Path>, home: &Path) -> (i32, Value) {
    let fake = FakeGithub::start();
    let mut cmd = bosun();
    cmd.env("BOSUN_GITHUB_API", fake.url())
        .env("HOME", home)
        .env_remove("CLAUDE_CONFIG_DIR");
    if let Some(c) = config {
        cmd.env("CLAUDE_CONFIG_DIR", c);
    }
    let out = cmd.arg("-C").arg(repo).arg("doctor").output().unwrap();
    let v = one_object(&out.stdout);
    (out.status.code().unwrap(), v["data"]["items"][12].clone())
}

fn repo(settings_ref: &str, caveman_on: bool) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "docs/setup-script.sh",
        &format!("KEELSTACK_MARKETPLACE_SHA={M}\nSUPERPOWERS_SHA={P}\nCAVEMAN_SHA={C}\n"),
    );
    write(
        tmp.path(),
        ".claude/settings.json",
        &format!(
            r#"{{"extraKnownMarketplaces":{{"keelstack":{{"source":{{"source":"github","repo":"keelstackhq/bosun","ref":"{settings_ref}"}}}}}},
 "enabledPlugins":{{"superpowers@keelstack":true,"caveman@keelstack":{caveman_on}}}}}"#
        ),
    );
    tmp
}

fn install(dir: &Path, key: &str, sp: &str) {
    write(
        dir,
        "plugins/installed_plugins.json",
        &format!(
            r#"{{"version":2,"plugins":{{"{key}":[{{"gitCommitSha":"{sp}"}}],"caveman@keelstack":[{{"gitCommitSha":"{C}"}}]}}}}"#
        ),
    );
}

#[test]
fn matching_pins_and_install_are_ok() {
    let (r, cfg, home) = (
        repo(M, true),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    install(cfg.path(), "superpowers@keelstack", P);
    let (_, it) = step13(r.path(), Some(cfg.path()), home.path());
    assert_eq!(
        (it["step"].clone(), it["name"].clone()),
        (13.into(), "plugins".into())
    );
    assert_eq!(it["status"], "ok", "{it}");
}

#[test]
fn the_install_is_read_from_home_without_claude_config_dir() {
    let (r, home) = (repo(M, true), tempfile::tempdir().unwrap());
    install(&home.path().join(".claude"), "superpowers@keelstack", P);
    let (_, it) = step13(r.path(), None, home.path());
    assert_eq!(it["status"], "ok", "{it}");
}

#[test]
fn a_missing_install_is_missing_and_exits_5() {
    let (r, cfg, home) = (
        repo(M, true),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let (code, it) = step13(r.path(), Some(cfg.path()), home.path());
    assert_eq!(it["status"], "missing", "{it}");
    assert_eq!(code, 5);
    assert!(
        it["detail"]
            .as_str()
            .unwrap()
            .contains("installed_plugins.json is missing")
    );
    assert!(!it["fix"].as_str().unwrap().is_empty());
}

#[test]
fn a_moved_settings_ref_is_missing_with_a_fix() {
    let other = "d".repeat(40);
    let (r, cfg, home) = (
        repo(&other, true),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    install(cfg.path(), "superpowers@keelstack", P);
    let (_, it) = step13(r.path(), Some(cfg.path()), home.path());
    assert_eq!(it["status"], "missing", "{it}");
    assert!(
        it["detail"]
            .as_str()
            .unwrap()
            .contains("KEELSTACK_MARKETPLACE_SHA")
    );
    assert!(
        it["fix"]
            .as_str()
            .unwrap()
            .contains(".claude/settings.json")
    );
}

#[test]
fn a_disabled_plugin_and_an_old_marketplace_install_are_missing() {
    let (r, cfg, home) = (
        repo(M, false),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    install(cfg.path(), "superpowers@superpowers-marketplace", P);
    let (_, it) = step13(r.path(), Some(cfg.path()), home.path());
    assert_eq!(it["status"], "missing", "{it}");
    let d = it["detail"].as_str().unwrap();
    assert!(d.contains("caveman@keelstack is not enabled"), "{d}");
    assert!(d.contains("superpowers@keelstack is not installed"), "{d}");
    assert!(it["fix"].as_str().unwrap().contains("@keelstack"));
}

#[test]
fn an_installed_plugin_off_its_pin_is_missing() {
    let (r, cfg, home) = (
        repo(M, true),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    install(cfg.path(), "superpowers@keelstack", "abc");
    let (_, it) = step13(r.path(), Some(cfg.path()), home.path());
    assert_eq!(it["status"], "missing", "{it}");
    assert!(it["detail"].as_str().unwrap().contains("SUPERPOWERS_SHA"));
}

#[test]
fn a_marketplace_file_off_the_script_pin_is_missing() {
    let (r, cfg, home) = (
        repo(M, true),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    install(cfg.path(), "superpowers@keelstack", P);
    let other = "e".repeat(40);
    write(
        r.path(),
        ".claude-plugin/marketplace.json",
        &format!(
            r#"{{"name":"keelstack","plugins":[{{"name":"superpowers","source":{{"source":"github","repo":"obra/superpowers","sha":"{P}"}}}},{{"name":"caveman","source":{{"source":"github","repo":"akramsystems/caveman","sha":"{other}"}}}}]}}"#
        ),
    );
    let (_, it) = step13(r.path(), Some(cfg.path()), home.path());
    assert_eq!(it["status"], "missing", "{it}");
    assert!(
        it["detail"]
            .as_str()
            .unwrap()
            .contains("marketplace.json pins caveman")
    );
}

#[test]
fn a_repo_without_a_plugin_block_is_missing() {
    let (tmp, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (_, it) = step13(tmp.path(), None, home.path());
    assert_eq!(it["status"], "missing", "{it}");
}
