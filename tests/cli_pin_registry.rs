//! `BOSUN_NPM_REGISTRY` must be https, or http on this machine (spec D35).

mod common;

use common::{bosun, one_object};

#[test]
fn a_plain_http_registry_off_this_machine_is_refused() {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("docs")).unwrap();
    let mut s = String::new();
    for t in ["SPLICE", "HOIST", "BERTH", "BOSUN"] {
        s += &format!(
            "{t}_VERSION=0.1.0\n{t}_SHA512=x\n{t}_LINUX_X64_SHA512=x\n{t}_LINUX_ARM64_SHA512=x\n"
        );
    }
    std::fs::write(d.path().join("docs/setup-script.sh"), &s).unwrap();
    for base in [
        "http://registry.example.invalid",
        "http://127.0.0.1@evil.invalid",
        "ftp://x",
    ] {
        let out = bosun()
            .env("BOSUN_NPM_REGISTRY", base)
            .args([
                "-C",
                d.path().to_str().unwrap(),
                "pin",
                "--latest",
                "--check",
            ])
            .output()
            .unwrap();
        let v = one_object(&out.stdout);
        assert_eq!(out.status.code(), Some(1), "{base}: {v}");
        let msg = v["error"]["message"].as_str().unwrap();
        assert!(
            msg.contains("BOSUN_NPM_REGISTRY must be an https:// URL"),
            "{base}: {msg}"
        );
    }
    assert_eq!(
        std::fs::read_to_string(d.path().join("docs/setup-script.sh")).unwrap(),
        s
    );
}
