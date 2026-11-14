//! `bosun doctor` reports "update available" (step 17) with the one command that fixes it.
mod common;

use common::fake::FakeGithub;
use common::npm::{Registry, repo};
use common::{bosun, one_object};
use serde_json::Value;

fn doctor(latest: &[(&str, &str)], check: bool) -> Value {
    let d = repo();
    let reg = Registry::start(latest, None, None);
    let fake = FakeGithub::start();
    let mut cmd = bosun();
    cmd.env("BOSUN_NPM_REGISTRY", &reg.base)
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_NOW", "2026-11-03T10:00:00Z");
    if check {
        cmd.env_remove("BOSUN_PIN_CHECK");
    }
    let out = cmd
        .arg("-C")
        .arg(d.path())
        .args(["doctor", "--full"])
        .output()
        .unwrap();
    one_object(&out.stdout)
}

fn update(v: &Value) -> Option<Value> {
    v["data"]["items"]
        .as_array()?
        .iter()
        .find(|i| i["step"] == 17)
        .cloned()
}

const BEHIND: [(&str, &str); 4] = [
    ("splice", "0.4.0"),
    ("hoist", "0.4.1"),
    ("berth", "0.1.0"),
    ("bosun", "99.0.0"),
];

#[test]
fn behind_pins_and_an_old_binary_are_an_update_warning() {
    let v = doctor(&BEHIND, true);
    let it = update(&v).expect("step 17");
    assert_eq!(it["status"], "warn", "{it}");
    assert_eq!(it["name"], "update");
    let detail = it["detail"].as_str().unwrap();
    assert!(detail.contains("hoist 0.4.0 -> 0.4.1"), "{detail}");
    assert!(detail.contains("bosun 0.9.0 -> 99.0.0"), "{detail}");
    assert!(
        detail.contains("npm i -g @keelstackhq/bosun@99.0.0"),
        "{detail}"
    );
    assert_eq!(it["fix"], "bosun pin --latest --pr");
}

#[test]
fn only_the_binary_behind_points_at_npm_not_at_a_pin_pr() {
    let d = repo();
    // Every pin already at npm latest, bosun's too; this binary (0.11) is older.
    let sp = d.path().join("docs/setup-script.sh");
    let text = std::fs::read_to_string(&sp).unwrap();
    std::fs::write(
        &sp,
        text.replace("BOSUN_VERSION=0.9.0", "BOSUN_VERSION=99.0.0"),
    )
    .unwrap();
    std::fs::write(d.path().join(".bosun/version"), "99.0.0\n").unwrap();
    let reg = Registry::start(
        &[
            ("splice", "0.4.0"),
            ("hoist", "0.4.0"),
            ("berth", "0.1.0"),
            ("bosun", "99.0.0"),
        ],
        None,
        None,
    );
    let fake = FakeGithub::start();
    let out = bosun()
        .env("BOSUN_NPM_REGISTRY", &reg.base)
        .env("BOSUN_GITHUB_API", fake.url())
        .env_remove("BOSUN_PIN_CHECK")
        .arg("-C")
        .arg(d.path())
        .args(["doctor", "--full"])
        .output()
        .unwrap();
    let it = update(&one_object(&out.stdout)).expect("step 17");
    assert_eq!(it["status"], "warn", "{it}");
    assert_eq!(it["fix"], "npm i -g @keelstackhq/bosun@99.0.0");
    assert!(!it["detail"].as_str().unwrap().contains("pin PR"), "{it}");
}

#[test]
fn up_to_date_is_silent() {
    let v = doctor(
        &[
            ("splice", "0.4.0"),
            ("hoist", "0.4.0"),
            ("berth", "0.1.0"),
            ("bosun", "0.9.0"),
        ],
        true,
    );
    let it = update(&v).expect("always an item");
    assert_eq!(it["status"], "ok", "{it}");
    assert_eq!(it["detail"], "pins and binary match npm latest");
}

#[test]
fn the_opt_out_is_a_skipped_item_with_the_reason() {
    let v = doctor(&BEHIND, false);
    let it = update(&v).expect("always an item");
    assert_eq!(it["status"], "skipped", "{it}");
    assert!(it["detail"].as_str().unwrap().contains("BOSUN_PIN_CHECK=0"));
}

#[test]
fn a_channel_repo_still_checks_the_binary() {
    let d = repo();
    std::fs::create_dir_all(d.path().join(".splice/channels")).unwrap();
    std::fs::write(d.path().join(".splice/channels/bosun.toml"), "# c\n").unwrap();
    let reg = Registry::start(&BEHIND, None, None);
    let fake = FakeGithub::start();
    let out = bosun()
        .env("BOSUN_NPM_REGISTRY", &reg.base)
        .env("BOSUN_GITHUB_API", fake.url())
        .env_remove("BOSUN_PIN_CHECK")
        .arg("-C")
        .arg(d.path())
        .args(["doctor", "--full"])
        .output()
        .unwrap();
    let it = update(&one_object(&out.stdout)).expect("item");
    assert_eq!(it["status"], "warn", "{it}");
    assert_eq!(it["fix"], "npm i -g @keelstackhq/bosun@99.0.0");
}

#[test]
fn an_unreachable_registry_does_not_fail_doctor() {
    let d = repo();
    let fake = FakeGithub::start();
    let out = bosun()
        .env("BOSUN_NPM_REGISTRY", "http://127.0.0.1:9")
        .env("BOSUN_GITHUB_API", fake.url())
        .env_remove("BOSUN_PIN_CHECK")
        .arg("-C")
        .arg(d.path())
        .args(["doctor", "--full"])
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    let it = update(&v).expect("always an item");
    assert_eq!(it["status"], "skipped", "{it}");
    assert!(
        it["detail"].as_str().unwrap().contains("npm unreachable"),
        "{it}"
    );
    assert!(v["data"]["items"].is_array());
}

fn doctor_in(d: &tempfile::TempDir, latest: &[(&str, &str)]) -> Value {
    let reg = Registry::start(latest, None, None);
    let fake = FakeGithub::start();
    let out = bosun()
        .env("BOSUN_NPM_REGISTRY", &reg.base)
        .env("BOSUN_GITHUB_API", fake.url())
        .env_remove("BOSUN_PIN_CHECK")
        .arg("-C")
        .arg(d.path())
        .args(["doctor", "--full"])
        .output()
        .unwrap();
    one_object(&out.stdout)
}

#[test]
fn a_broken_script_is_not_called_a_network_error() {
    let d = repo();
    let sp = d.path().join("docs/setup-script.sh");
    let text = std::fs::read_to_string(&sp).unwrap();
    std::fs::write(&sp, format!("{text}HOIST_VERSION=0.4.0\n")).unwrap();
    // The binary (0.11) is not behind npm's bosun (0.9), so the broken script is the only signal.
    let latest = [
        ("splice", "0.4.0"),
        ("hoist", "0.4.0"),
        ("berth", "0.1.0"),
        ("bosun", "0.9.0"),
    ];
    let it = update(&doctor_in(&d, &latest)).expect("item");
    assert_eq!(it["status"], "skipped", "{it}");
    let detail = it["detail"].as_str().unwrap();
    assert!(
        detail.contains("pin check failed") && !detail.contains("unreachable"),
        "{it}"
    );
    assert!(!it["fix"].as_str().unwrap().contains("network"), "{it}");
}

#[test]
fn a_pin_error_does_not_hide_an_old_binary() {
    // hoist's npm latest (0.3.0) is older than its pin (0.4.0): the pin compare refuses.
    let d = repo();
    let latest = [
        ("splice", "0.4.0"),
        ("hoist", "0.3.0"),
        ("berth", "0.1.0"),
        ("bosun", "99.0.0"),
    ];
    let it = update(&doctor_in(&d, &latest)).expect("item");
    assert_eq!(it["status"], "warn", "{it}");
    assert_eq!(it["fix"], "npm i -g @keelstackhq/bosun@99.0.0");
}
