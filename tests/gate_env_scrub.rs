//! A gate's child process never inherits the GitHub token (a repo's test command is repo code).
#![cfg(unix)]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use common::{TOKEN_CANARY, bosun};

#[test]
fn the_gate_child_has_no_github_token() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap().join("repo");
    fs::create_dir_all(&root).unwrap();
    let git = |a: &[&str]| {
        let o = Command::new("git")
            .current_dir(&root)
            .args(a)
            .output()
            .unwrap();
        assert!(o.status.success(), "{a:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.name", "t"]);
    git(&["config", "user.email", "t@example.com"]);
    fs::write(root.join("Cargo.toml"), "[package]\nname = \"a\"\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "base"]);
    git(&["checkout", "-q", "-b", "work"]);
    fs::write(root.join("x.rs"), "//\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "c"]);
    let bin = tmp.path().join("bin");
    fs::create_dir_all(&bin).unwrap();
    let seen = tmp.path().join("env.log");
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = metadata ]; then printf '{{\"packages\":[]}}'; exit 0; fi\n\
         echo \"gh=[$GH_TOKEN] github=[$GITHUB_TOKEN]\" >> {}\nexit 0\n",
        seen.display()
    );
    let cargo = bin.join("cargo");
    fs::write(&cargo, script).unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
    let out = bosun()
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("GH_TOKEN", TOKEN_CANARY)
        .env_remove("GITHUB_BASE_REF")
        .env_remove("CI")
        .arg("-C")
        .arg(&root)
        .args(["gate", "test", "--base", "main", "--all"])
        .output()
        .unwrap();
    let log = fs::read_to_string(&seen).unwrap_or_default();
    assert!(
        log.contains("gh=[] github=[]"),
        "child saw a token or never ran: {log:?} {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
