mod common;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;
use std::path::Path;

const M1: &str = "5c9072877417873fee42be1805aa776a453ec394";
const P1: &str = "22f0b463e7a5e579904dd6211a520321035526f7";
const C: &str = "a9cc1f7b82598852e8c6b8e599dde6ac2491b26e";

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn step13(repo: &Path, home: &Path) -> Value {
    let fake = FakeGithub::start();
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("HOME", home)
        .arg("-C")
        .arg(repo)
        .arg("doctor")
        .output()
        .unwrap();
    one_object(&out.stdout)["data"]["items"][12].clone()
}

fn repo(marketplace_ref: &str, caveman_on: bool) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "docs/setup-script.sh",
        &format!("SUPERPOWERS_MARKETPLACE_SHA={M1}\nSUPERPOWERS_SHA={P1}\nCAVEMAN_SHA={C}\n"),
    );
    write(
        tmp.path(),
        ".claude/settings.json",
        &format!(
            r#"{{"extraKnownMarketplaces":{{
  "superpowers-marketplace":{{"source":{{"source":"github","repo":"a/b","ref":"{marketplace_ref}"}}}},
  "akramsystems-plugins":{{"source":{{"source":"github","repo":"c/d","ref":"{C}"}}}}}},
 "enabledPlugins":{{"superpowers@superpowers-marketplace":true,"caveman@akramsystems-plugins":{caveman_on}}}}}"#
        ),
    );
    tmp
}

#[test]
fn matching_pins_without_an_install_are_ok_with_a_note() {
    let (r, home) = (repo(M1, true), tempfile::tempdir().unwrap());
    let it = step13(r.path(), home.path());
    assert_eq!(it["step"], 13);
    assert_eq!(it["name"], "plugins");
    assert_eq!(it["status"], "ok", "{it}");
    assert!(
        it["detail"]
            .as_str()
            .unwrap()
            .contains("not installed in this container")
    );
}

#[test]
fn a_moved_marketplace_ref_is_missing_with_a_fix() {
    let (r, home) = (repo("deadbeef", true), tempfile::tempdir().unwrap());
    let it = step13(r.path(), home.path());
    assert_eq!(it["status"], "missing", "{it}");
    assert!(
        it["detail"]
            .as_str()
            .unwrap()
            .contains("SUPERPOWERS_MARKETPLACE_SHA")
    );
    assert!(!it["fix"].as_str().unwrap().is_empty());
}

#[test]
fn a_disabled_plugin_and_an_installed_plugin_off_its_pin_are_notes() {
    let (r, home) = (repo(M1, false), tempfile::tempdir().unwrap());
    write(
        home.path(),
        ".claude/plugins/installed_plugins.json",
        &format!(
            r#"{{"version":2,"plugins":{{"superpowers@superpowers-marketplace":[{{"gitCommitSha":"abc"}}],"caveman@akramsystems-plugins":[{{"gitCommitSha":"{C}"}}]}}}}"#
        ),
    );
    let it = step13(r.path(), home.path());
    assert_eq!(it["status"], "ok", "{it}");
    let d = it["detail"].as_str().unwrap();
    assert!(
        d.contains("SUPERPOWERS_SHA") && d.contains("caveman@akramsystems-plugins"),
        "{d}"
    );
}

#[test]
fn a_repo_without_a_plugin_block_is_ok() {
    let (tmp, home) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let it = step13(tmp.path(), home.path());
    assert_eq!(it["status"], "ok", "{it}");
}
