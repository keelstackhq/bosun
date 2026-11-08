mod common;

use common::{bosun, one_object};
use serde_json::Value;
use std::path::Path;

fn adopt(dir: &Path, extra: &[&str]) -> Value {
    let out = bosun()
        .env_remove("GITHUB_TOKEN")
        .env_remove("BOSUN_REPO")
        .args(["-C", dir.to_str().unwrap(), "adopt", "--no-remote"])
        .args(extra)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    one_object(&out.stdout)
}

fn steps(v: &Value) -> String {
    v["data"]["owner_steps"].to_string()
}

fn commands(v: &Value) -> Vec<String> {
    v["data"]["commands"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn owner_flag_drops_the_owners_step_and_key_step_is_exact() {
    let t = tempfile::tempdir().unwrap();
    let v = adopt(t.path(), &["--owner", "alice"]);
    let s = steps(&v);
    assert!(!s.contains("governance/owners.toml"), "{s}");
    assert!(s.contains("bosun keygen --enroll"), "{s}");
}

#[test]
fn no_owner_says_to_rerun_with_owner() {
    let t = tempfile::tempdir().unwrap();
    let s = steps(&adopt(t.path(), &[]));
    assert!(s.contains("bosun adopt --owner <your-github-login>"), "{s}");
}

#[test]
fn existing_owner_and_key_leave_neither_step() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join("governance")).unwrap();
    std::fs::create_dir_all(t.path().join(".bosun")).unwrap();
    std::fs::write(
        t.path().join("governance/owners.toml"),
        "[[owner]]\ngithub = \"bob\"\n",
    )
    .unwrap();
    std::fs::write(
        t.path().join(".bosun/approvers.toml"),
        "[[key]]\nid = \"bob\"\n",
    )
    .unwrap();
    let s = steps(&adopt(t.path(), &[]));
    assert!(!s.contains("--owner"), "{s}");
    assert!(!s.contains("keygen"), "{s}");
}

#[test]
fn git_add_lists_what_adopt_wrote() {
    let t = tempfile::tempdir().unwrap();
    let c = commands(&adopt(t.path(), &["--owner", "alice"]));
    assert_eq!(
        c[0],
        "git switch bosun-setup 2>/dev/null || git switch -c bosun-setup"
    );
    let add: Vec<&str> = c[1].strip_prefix("git add ").unwrap().split(' ').collect();
    for p in [
        ".bosun/adopt.toml",
        ".bosun/approvers.toml",
        "docs/setup-script.sh",
        "governance/owners.toml",
    ] {
        assert!(add.contains(&p), "{p} in {add:?}");
    }
    assert!(
        add.iter().all(|p| p.contains('.') || p.contains('/')),
        "{add:?}"
    );
    assert!(!add.contains(&".bosun"), "{add:?}");
    let mut sorted = add.clone();
    sorted.sort_unstable();
    assert_eq!(add, sorted);
    assert_eq!(c[2], "git commit -m \"chore: set up bosun\"");
    assert_eq!(c[3], "git push -u origin bosun-setup");
    assert_eq!(c[4], "bosun doctor");
}

#[test]
fn branch_protection_link_from_a_github_origin() {
    let t = tempfile::tempdir().unwrap();
    let d = t.path().to_str().unwrap();
    let git = |a: &[&str]| {
        let st = std::process::Command::new("git")
            .args(["-C", d])
            .args(a)
            .status()
            .unwrap();
        assert!(st.success());
    };
    git(&["init", "-q"]);
    let plain = steps(&adopt(t.path(), &["--owner", "alice"]));
    assert!(!plain.contains("settings/branches"), "{plain}");
    git(&[
        "remote",
        "add",
        "origin",
        "https://github.com/acme/widgets.git",
    ]);
    let s = steps(&adopt(t.path(), &["--owner", "alice"]));
    assert!(
        s.contains("https://github.com/acme/widgets/settings/branches"),
        "{s}"
    );
}

#[test]
fn a_rerun_with_nothing_new_suggests_no_commit() {
    let t = tempfile::tempdir().unwrap();
    adopt(t.path(), &["--owner", "alice"]);
    let c = commands(&adopt(t.path(), &["--owner", "alice"]));
    assert_eq!(c, vec!["bosun doctor".to_string()], "{c:?}");
}

#[test]
fn spaced_toml_headers_count_as_listed() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join("governance")).unwrap();
    std::fs::create_dir_all(t.path().join(".bosun")).unwrap();
    std::fs::write(
        t.path().join("governance/owners.toml"),
        "[[ owner ]] # me\ngithub = \"bob\"\n",
    )
    .unwrap();
    std::fs::write(
        t.path().join(".bosun/approvers.toml"),
        "[[ key ]]\nid = \"bob\"\ned25519 = \"x\"\n",
    )
    .unwrap();
    let s = steps(&adopt(t.path(), &[]));
    assert!(!s.contains("governance/owners.toml"), "{s}");
    assert!(!s.contains("bosun keygen"), "{s}");
}

#[test]
fn a_kept_owners_file_without_an_owner_says_to_edit_it() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join("governance")).unwrap();
    std::fs::write(t.path().join("governance/owners.toml"), "# none yet\n").unwrap();
    let s = steps(&adopt(t.path(), &["--owner", "alice"]));
    assert!(!s.contains("Rerun with"), "{s}");
    assert!(s.contains("github = \\\"alice\\\""), "{s}");
}

#[test]
fn dry_run_has_no_commands_and_steps_name_the_setup_snippet() {
    let t = tempfile::tempdir().unwrap();
    let v = adopt(t.path(), &["--owner", "alice", "--dry-run"]);
    assert_eq!(v["data"]["commands"], serde_json::json!([]));
    assert!(steps(&v).contains("bosun setup-snippet"), "{}", steps(&v));
}

#[test]
fn an_owners_file_without_owner_gets_the_edit_step_on_the_first_run() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join("governance")).unwrap();
    std::fs::write(t.path().join("governance/owners.toml"), "# none yet\n").unwrap();
    let s = steps(&adopt(t.path(), &[]));
    assert!(!s.contains("Rerun with"), "{s}");
    assert!(s.contains("<your-github-login>"), "{s}");
}
