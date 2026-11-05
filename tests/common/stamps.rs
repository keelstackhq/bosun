//! Local gate stamps for `bosun pr --ready`: what `bosun gate` writes after a pass.
#![allow(dead_code)]

use std::path::Path;
use std::process::Command;

use super::fake::FakeGithub;

/// The gates the default (rust) stack must have passed before a PR is marked ready.
pub const READY_GATES: [&str; 5] = ["test-integrity", "fmt", "lint", "test", "deps"];

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

pub fn head_sha(dir: &Path) -> String {
    git(dir, &["rev-parse", "HEAD"])
}

/// The directory of the stamps of the tree at HEAD.
pub fn stamp_dir(dir: &Path) -> std::path::PathBuf {
    let common = git(
        dir,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    );
    let tree = git(dir, &["rev-parse", "HEAD^{tree}"]);
    Path::new(&common).join("bosun/gates").join(tree)
}

/// Stamp the given gates for the tree at HEAD.
pub fn stamp(dir: &Path, gates: &[&str]) {
    let sd = stamp_dir(dir);
    std::fs::create_dir_all(&sd).unwrap();
    for g in gates {
        std::fs::write(sd.join(g), "ok\n").unwrap();
    }
}

/// Stamp every ready gate of the default stack for the tree at HEAD.
pub fn stamp_all(dir: &Path) {
    stamp(dir, &READY_GATES);
}

/// Make PR `number` ready to leave draft: its head is the local HEAD and every gate is stamped.
pub fn ready(fake: &FakeGithub, dir: &Path, number: u64) {
    fake.pull_head_sha(number, &head_sha(dir));
    stamp_all(dir);
}

/// Like `ready` for every pull request of the fake: the gates passed locally on the pushed head.
pub fn ready_all(fake: &FakeGithub, dir: &Path) {
    let numbers: Vec<u64> = {
        let s = fake.state.lock().unwrap();
        s.pulls
            .iter()
            .filter_map(|p| p["number"].as_u64())
            .collect()
    };
    for n in numbers {
        fake.pull_head_sha(n, &head_sha(dir));
    }
    stamp_all(dir);
}
