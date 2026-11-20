//! `.bosun/pins.toml` is the one source of the rust fallback and the nextest and cargo-deny
//! versions: the templates and the setup scripts read it and hard-code none of them.

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

const READERS: [&str; 5] = [
    "channel/pieces/core/files/.github/workflows/bosun.yml.jinja",
    "channel/pieces/circleci/files/.circleci/src/jobs/bosun-ok.yml.jinja",
    "channel/pieces/core/files/docs/setup-script.sh.jinja",
    "docs/setup-script.sh",
    "channel/pieces/core/files/.bosun/pins.toml",
];

fn pin(text: &str, section: &str, key: &str) -> String {
    let mut on = false;
    for l in text.lines() {
        let l = l.trim();
        if l.starts_with('[') {
            on = l == format!("[{section}]");
        } else if on
            && let Some((k, v)) = l.split_once('=')
            && k.trim() == key
        {
            return v.trim().trim_matches('"').to_string();
        }
    }
    panic!("pins.toml has no [{section}] {key}");
}

#[test]
fn pins_toml_has_the_keys_and_the_channel_copy_matches_bosuns_own() {
    let own = read(".bosun/pins.toml");
    assert_eq!(
        own,
        read(READERS[4]),
        "the channel copy and .bosun/pins.toml differ"
    );
    assert_eq!(pin(&own, "rust", "fallback"), "1.97");
    for tool in ["nextest", "cargo-deny"] {
        let v = pin(&own, tool, "version");
        assert_eq!(v.split('.').count(), 3, "{tool} {v}");
        for k in ["linux_x64_sha256", "linux_arm64_sha256"] {
            let h = pin(&own, tool, k);
            assert!(
                h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()),
                "{tool} {k}"
            );
        }
    }
    let piece = read("channel/pieces/core/piece.toml");
    assert!(
        piece.contains("\".bosun/pins.toml\""),
        "pins.toml is a channel file"
    );
}

#[test]
fn no_template_or_setup_script_hard_codes_a_pin() {
    let pins = read(".bosun/pins.toml");
    let mut needles = vec![pin(&pins, "rust", "fallback")];
    for t in ["nextest", "cargo-deny"] {
        needles.push(pin(&pins, t, "version"));
        needles.push(pin(&pins, t, "linux_x64_sha256"));
        needles.push(pin(&pins, t, "linux_arm64_sha256"));
    }
    // The old circleci pin, gone with its file.
    needles.push("0.9.148".into());
    for rel in &READERS[..4] {
        let text = read(rel);
        for n in &needles {
            // "1.97" as a bare number would also match inside other versions: look for the
            // fallback only as a quoted or toolchain-argument value.
            let hit = if n == "1.97" {
                text.contains("install 1.97")
                    || text.contains("default 1.97")
                    || text.contains("ch=1.97")
            } else {
                text.contains(n.as_str())
            };
            assert!(!hit, "{rel} hard-codes {n}");
        }
    }
}

#[test]
fn setup_script_and_both_ci_templates_read_pins_toml() {
    for rel in &READERS[..4] {
        assert!(
            read(rel).contains(".bosun/pins.toml"),
            "{rel} does not read .bosun/pins.toml"
        );
    }
    let wf = read(READERS[0]);
    assert!(
        wf.contains("cargo-nextest@"),
        "bosun.yml installs an unpinned nextest"
    );
    assert!(
        wf.contains("cargo-deny@"),
        "bosun.yml installs an unpinned cargo-deny"
    );
    assert!(
        Path::new(&root().join("channel/pieces/core/files/.bosun/pins.toml")).is_file(),
        "channel copy"
    );
}

#[test]
fn hull_pins_are_preferred_when_the_repo_has_them() {
    for rel in [READERS[2], READERS[3]] {
        let t = read(rel);
        assert!(t.contains("scripts/pins.mjs --shell"), "{rel}");
        assert!(t.contains("scripts/proxy-ca.sh install"), "{rel}");
    }
    let wf = read(READERS[0]);
    assert!(
        wf.contains("git show \"$BASE:pins.toml\""),
        "base copy only"
    );
    assert!(wf.contains("pin pins cargo_deny"), "{wf}");
}
