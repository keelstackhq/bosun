//! `bosun setup-snippet` and the bootstrap it prints (spec D23): pasted once, it runs each
//! checkout's own docs/setup-script.sh and never fails the session.
mod common;

use common::{bosun, one_object};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

fn snippet() -> String {
    let out = bosun().arg("setup-snippet").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v = one_object(&out.stdout);
    assert_eq!(v["command"], "setup-snippet");
    assert!(
        v["data"]["client_message"]
            .as_str()
            .unwrap()
            .contains("once")
    );
    v["data"]["script"].as_str().unwrap().to_string()
}

#[test]
fn the_bootstrap_is_short_and_pins_nothing() {
    let s = snippet();
    assert!(s.starts_with("#!/usr/bin/env bash\n"), "{s}");
    assert!(s.lines().count() <= 40, "{} lines", s.lines().count());
    for pin in ["_SHA512=", "_VERSION=", "@0.", "set -e"] {
        assert!(!s.contains(pin), "the bootstrap must not contain {pin}");
    }
    assert!(s.trim_end().ends_with("exit 0"));
}

#[test]
fn the_readme_shows_the_exact_bootstrap() {
    let readme = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("README.md"));
    assert!(
        readme.unwrap().contains(&snippet()),
        "README.md: paste `bosun setup-snippet`"
    );
}

fn stub(dir: &Path, name: &str, body: &str) {
    let p = dir.join(name);
    fs::write(&p, format!("#!/bin/bash\n{body}\n")).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A PATH with only the tools the bootstrap needs, so no real bosun or npm leaks in.
fn tools(dir: &Path) {
    for t in [
        "bash", "timeout", "rm", "ln", "cat", "pwd", "mkdir", "node", "base64", "printf",
    ] {
        let real = ["/usr/bin", "/bin"]
            .iter()
            .map(|d| Path::new(d).join(t))
            .find(|p| p.exists());
        std::os::unix::fs::symlink(real.unwrap(), dir.join(t)).unwrap();
    }
    stub(dir, "uname", "echo x86_64");
}

fn checkout(root: &Path, name: &str, git: bool, script: Option<&str>) {
    let d = root.join(name);
    fs::create_dir_all(d.join("docs")).unwrap();
    if git {
        fs::create_dir_all(d.join(".git")).unwrap();
        fs::create_dir_all(d.join(".bosun")).unwrap();
        fs::write(d.join(".bosun/version"), "0.9.0\n").unwrap();
    }
    if let Some(s) = script {
        fs::write(d.join("docs/setup-script.sh"), s).unwrap();
    }
}

/// Run the bootstrap in `work`; returns (exit code, stderr, log).
fn boot(work: &Path, bin: &Path, log: &Path) -> (i32, String, String) {
    let out = Command::new(bin.join("bash"))
        .arg("-c")
        .arg(snippet())
        .current_dir(work)
        .env_clear()
        .env("PATH", bin)
        .env("LOG", log)
        .env("BOSUN_BOOTSTRAP_PREFIX", work.join("prefix"))
        .env("BOSUN_BOOTSTRAP_BIN", bin)
        .output()
        .unwrap();
    let l = fs::read_to_string(log).unwrap_or_default();
    (
        out.status.code().unwrap(),
        String::from_utf8_lossy(&out.stderr).into(),
        l,
    )
}

#[test]
fn each_checkout_runs_its_own_script_and_a_failure_only_warns() {
    let tmp = tempfile::tempdir().unwrap();
    let (work, bin, log) = (
        tmp.path().join("w"),
        tmp.path().join("bin"),
        tmp.path().join("log"),
    );
    fs::create_dir_all(&bin).unwrap();
    tools(&bin);
    stub(&bin, "npm", "echo \"npm $*\" >>\"$LOG\"");
    checkout(
        &work,
        "a",
        true,
        Some("echo \"a $(pwd)\" >>\"$LOG\"; read -r x && echo stdin >>\"$LOG\""),
    );
    checkout(&work, "b", true, Some("echo b >>\"$LOG\"; exit 7"));
    checkout(&work, "c", true, None);
    checkout(&work, "d", false, Some("echo d >>\"$LOG\""));
    checkout(&work, "e", true, Some("echo e >>\"$LOG\""));
    fs::remove_file(work.join("e/.bosun/version")).unwrap();
    let (code, err, l) = boot(&work, &bin, &log);
    assert_eq!(code, 0, "{err}");
    assert!(
        l.contains(&format!("a {}", work.join("a").display())),
        "{l}"
    );
    assert!(
        l.contains("b\n") && !l.contains("d\n") && !l.contains("stdin"),
        "{l}"
    );
    assert!(
        !l.contains("e\n"),
        "no .bosun/version: not a bosun checkout\n{l}"
    );
    assert!(
        err.contains("b/docs/setup-script.sh failed (exit 7)"),
        "{err}"
    );
    assert!(
        !l.contains("npm"),
        "a repo script exists: no fallback install\n{l}"
    );
}

#[test]
fn odd_checkout_names_still_run() {
    let tmp = tempfile::tempdir().unwrap();
    let (work, bin, log) = (
        tmp.path().join("w"),
        tmp.path().join("bin"),
        tmp.path().join("log"),
    );
    fs::create_dir_all(&bin).unwrap();
    tools(&bin);
    checkout(&work, "-x", true, Some("echo dash >>\"$LOG\""));
    checkout(&work, "a b", true, Some("echo space >>\"$LOG\""));
    let (code, err, l) = boot(&work, &bin, &log);
    assert_eq!(code, 0);
    assert!(l.contains("dash\n") && l.contains("space\n"), "{l}{err}");
    assert!(!err.contains("failed"), "{err}");
}

#[test]
fn the_current_dir_counts_as_a_checkout() {
    let tmp = tempfile::tempdir().unwrap();
    let (work, bin, log) = (
        tmp.path().join("w"),
        tmp.path().join("bin"),
        tmp.path().join("log"),
    );
    fs::create_dir_all(&bin).unwrap();
    tools(&bin);
    checkout(tmp.path(), "w", true, Some("echo here >>\"$LOG\""));
    let (code, _, l) = boot(&work, &bin, &log);
    assert_eq!((code, l.as_str()), (0, "here\n"));
}

const NPM_OK: &str = r#"echo "npm $*" >>"$LOG"
if [ "$1" = install ]; then
  m="$3/node_modules"; mkdir -p "$m/.bin" "$m/@keelstackhq/bosun"
  printf '#!/bin/bash\necho bosun 9.9.9\n' >"$m/.bin/bosun"; chmod +x "$m/.bin/bosun"
  printf '{"packages":{"":{},"node_modules/@keelstackhq/bosun":{"version":"9.9.9","integrity":"sha512-q83v"},"node_modules/@keelstackhq/bosun-darwin-x64":{"version":"9.9.9","integrity":"sha512-AAAA"}}}' >"$3/package-lock.json"
fi"#;

/// A fake attestations endpoint: serves a fixture (a signed statement for digest abcdef, `q83v`,
/// whose payload names keelstackhq/bosun), or fails like `curl -f` on a 404.
fn curl_stub(bin: &Path, fixture: Option<&str>) {
    let body = match fixture {
        Some(f) => format!(
            "cat '{}'",
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/bootstrap")
                .join(f)
                .display()
        ),
        None => "exit 22".into(),
    };
    stub(bin, "curl", &format!("echo \"curl $*\" >>\"$LOG\"\n{body}"));
}

#[test]
fn without_a_repo_script_the_latest_bosun_is_kept_only_with_signatures_and_bosun_provenance() {
    let good = Some("attestations-good.json");
    // (audit, attestations, lockfile integrity, kept)
    for (audit, att, integrity, kept) in [
        ("exit 0", good, "q83v", true),
        ("exit 1", good, "q83v", false),
        // The payload names keelstackhq/bosun, but the signing certificate is another repo's.
        ("exit 0", Some("attestations-evil-san.json"), "q83v", false),
        ("exit 0", good, "q83w", false),
        // The right certificate, but the payload changed after signing.
        ("exit 0", Some("attestations-bad-sig.json"), "q83v", false),
        ("exit 0", None, "q83v", false),
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let (work, bin, log) = (
            tmp.path().join("w"),
            tmp.path().join("bin"),
            tmp.path().join("log"),
        );
        fs::create_dir_all(&work).unwrap();
        fs::create_dir_all(&bin).unwrap();
        tools(&bin);
        let npm = NPM_OK.replace("sha512-q83v", &format!("sha512-{integrity}"));
        stub(
            &bin,
            "npm",
            &format!("{npm}\n[ \"$1\" = audit ] && {audit}\nexit 0"),
        );
        curl_stub(&bin, att);
        let (code, err, l) = boot(&work, &bin, &log);
        let case = format!("{audit} {att:?} {integrity}");
        assert_eq!(code, 0, "{case}: {err}");
        assert!(
            l.contains("@keelstackhq/bosun@latest") && l.contains("npm audit signatures"),
            "{l}"
        );
        assert_eq!(bin.join("bosun").exists(), kept, "{case}: {err}");
        assert_eq!(work.join("prefix").exists(), kept, "{case}");
        assert!(
            err.contains(if kept {
                "verified"
            } else {
                "no verified bosun"
            }),
            "{case}: {err}"
        );
        if audit == "exit 0" {
            // Only the installed package is checked, with a time limit, at registry.npmjs.org.
            assert!(
                l.contains("-m 30 https://registry.npmjs.org/-/npm/v1/attestations/%40keelstackhq%2Fbosun@9.9.9"),
                "{l}"
            );
            assert!(!l.contains("darwin"), "{l}");
        }
    }
}

#[test]
fn a_hung_repo_script_is_killed_after_the_grace() {
    let s = snippet();
    assert!(s.contains("timeout -k 10 1800 "), "{s}");
}

#[test]
fn an_installed_bosun_or_a_missing_npm_skips_the_fallback_quietly_or_with_a_warning() {
    let tmp = tempfile::tempdir().unwrap();
    let (work, bin, log) = (
        tmp.path().join("w"),
        tmp.path().join("bin"),
        tmp.path().join("log"),
    );
    fs::create_dir_all(&work).unwrap();
    fs::create_dir_all(&bin).unwrap();
    tools(&bin);
    let (code, err, _) = boot(&work, &bin, &log);
    assert_eq!(code, 0);
    assert!(err.contains("no verified bosun"), "{err}");
    stub(&bin, "bosun", "echo bosun 0.1.0");
    stub(&bin, "npm", "echo \"npm $*\" >>\"$LOG\"");
    let (code, err, l) = boot(&work, &bin, &log);
    assert_eq!((code, l.as_str(), err.as_str()), (0, "", ""));
}

#[test]
fn an_unknown_arch_warns_and_goes_on() {
    let tmp = tempfile::tempdir().unwrap();
    let (work, bin, log) = (
        tmp.path().join("w"),
        tmp.path().join("bin"),
        tmp.path().join("log"),
    );
    fs::create_dir_all(&bin).unwrap();
    tools(&bin);
    stub(&bin, "uname", "echo riscv64");
    checkout(tmp.path(), "w", true, Some("echo ran >>\"$LOG\""));
    let (code, err, l) = boot(&work, &bin, &log);
    assert_eq!((code, l.as_str()), (0, "ran\n"));
    assert!(err.contains("riscv64"), "{err}");
}
