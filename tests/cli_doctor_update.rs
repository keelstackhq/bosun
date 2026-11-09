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
    let out = cmd.arg("-C").arg(d.path()).arg("doctor").output().unwrap();
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
        .arg("doctor")
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
    assert!(update(&v).is_none(), "{v}");
}

#[test]
fn the_opt_out_skips_the_check() {
    let v = doctor(&BEHIND, false);
    assert!(update(&v).is_none(), "{v}");
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
        .arg("doctor")
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    assert!(update(&v).is_none(), "{v}");
    assert!(v["data"]["items"].is_array());
}
