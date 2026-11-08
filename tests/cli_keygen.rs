//! `bosun keygen`: make an approval key and optionally enroll it in .bosun/approvers.toml.
mod common;

use common::{bosun, one_object};
use serde_json::Value;
use std::path::Path;

fn git_repo() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    let ok = std::process::Command::new("git")
        .args(["init", "-q"])
        .arg(d.path())
        .status()
        .unwrap()
        .success();
    assert!(ok);
    d
}

/// Run `bosun -C cwd keygen <args>` with HOME set; returns (code, stdout text, envelope).
fn keygen(cwd: &Path, home: &Path, args: &[&str]) -> (i32, String, Value) {
    let out = bosun()
        .env("HOME", home)
        .arg("-C")
        .arg(cwd)
        .arg("keygen")
        .args(args)
        .output()
        .unwrap();
    let text = String::from_utf8(out.stdout.clone()).unwrap();
    let v = one_object(&out.stdout);
    if v["ok"] == false {
        assert!(!v["error"]["fix"].as_str().unwrap().trim().is_empty());
    }
    (out.status.code().unwrap(), text, v)
}

#[test]
fn writes_key_0600_and_reports_the_public_half() {
    let (repo, home, outd) = (
        git_repo(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    let pem = outd.path().join("k.pem");
    let (code, text, v) = keygen(repo.path(), home.path(), &["--out", pem.to_str().unwrap()]);
    assert_eq!(code, 0, "{text}");
    let d = &v["data"];
    assert_eq!(d["id"], "owner-2026-10");
    assert_eq!(d["role"], "owner");
    assert_eq!(d["enrolled"], false);
    assert_eq!(d["path"], pem.to_str().unwrap());
    let body = std::fs::read_to_string(&pem).unwrap();
    assert!(body.starts_with("-----BEGIN PRIVATE KEY-----\n"));
    assert!(body.trim_end().ends_with("-----END PRIVATE KEY-----"));
    assert!(body.lines().all(|l| l.len() <= 64));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&pem).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    // The toml block parses and carries the public key.
    let t: toml::Table = d["toml"].as_str().unwrap().parse().unwrap();
    let k = &t["key"].as_array().unwrap()[0];
    assert_eq!(
        k["ed25519"].as_str().unwrap(),
        d["public_key"].as_str().unwrap()
    );
    assert_eq!(k["role"].as_str().unwrap(), "owner");
    // No private key text on stdout.
    let secret: String = body.lines().filter(|l| !l.starts_with("-----")).collect();
    assert!(!text.contains("PRIVATE KEY") && !text.contains(&secret));
    assert!(!repo.path().join(".bosun").exists());
}

#[test]
fn default_out_is_under_home_and_never_overwritten() {
    let (repo, home) = (git_repo(), tempfile::tempdir().unwrap());
    let (code, _, v) = keygen(repo.path(), home.path(), &["--role", "delegate"]);
    assert_eq!(code, 0);
    assert_eq!(v["data"]["id"], "delegate-2026-10");
    let pem = home.path().join(".bosun/approver.pem");
    assert_eq!(v["data"]["path"], pem.to_str().unwrap());
    let before = std::fs::read(&pem).unwrap();
    let (code, _, v) = keygen(repo.path(), home.path(), &[]);
    assert_eq!(code, 3);
    assert!(v["error"]["fix"].as_str().unwrap().contains("--out"));
    assert_eq!(std::fs::read(&pem).unwrap(), before);
}

#[test]
fn refuses_out_inside_the_worktree() {
    let (repo, home) = (git_repo(), tempfile::tempdir().unwrap());
    let inside = repo.path().join("key.pem");
    let (code, _, v) = keygen(
        repo.path(),
        home.path(),
        &["--out", inside.to_str().unwrap()],
    );
    assert_eq!(code, 2);
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("~/.bosun/approver.pem")
    );
    assert!(!inside.exists());
}

#[test]
fn rejects_a_bad_id() {
    let (repo, home) = (git_repo(), tempfile::tempdir().unwrap());
    for id in ["", "a b", "x\"y", &"a".repeat(65)] {
        let (code, _, _) = keygen(repo.path(), home.path(), &["--id", id]);
        assert_eq!(code, 2, "id {id:?}");
    }
    assert!(!home.path().join(".bosun").exists());
}

#[test]
fn enroll_creates_then_appends_and_refuses_duplicates() {
    let (repo, home) = (git_repo(), tempfile::tempdir().unwrap());
    let a = home.path().join("a.pem");
    let b = home.path().join("b.pem");
    let (code, _, v) = keygen(
        repo.path(),
        home.path(),
        &["--out", a.to_str().unwrap(), "--id", "one", "--enroll"],
    );
    assert_eq!(code, 0);
    assert_eq!(v["data"]["enrolled"], true);
    let file = repo.path().join(".bosun/approvers.toml");
    let t: toml::Table = std::fs::read_to_string(&file).unwrap().parse().unwrap();
    assert_eq!(t["key"].as_array().unwrap().len(), 1);
    let steps = v["data"]["steps"].to_string();
    assert!(steps.contains("cannot enroll its own key") && steps.contains("--sign-key"));

    let (code, _, _) = keygen(
        repo.path(),
        home.path(),
        &["--out", b.to_str().unwrap(), "--id", "two", "--enroll"],
    );
    assert_eq!(code, 0);
    let t: toml::Table = std::fs::read_to_string(&file).unwrap().parse().unwrap();
    assert_eq!(t["key"].as_array().unwrap().len(), 2);

    // Same id again: conflict, and no new key file is left behind.
    let c = home.path().join("c.pem");
    let before = std::fs::read_to_string(&file).unwrap();
    let (code, _, v) = keygen(
        repo.path(),
        home.path(),
        &["--out", c.to_str().unwrap(), "--id", "one", "--enroll"],
    );
    assert_eq!(code, 3);
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert!(!c.exists());
    assert_eq!(std::fs::read_to_string(&file).unwrap(), before);
}
