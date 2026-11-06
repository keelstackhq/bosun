mod common;

use bosun::core::adopt::Manifest;
use common::fake::FakeGithub;
use common::run;
use serde_json::Value;
use std::path::Path;

const NOW: &str = "2026-11-06T10:00:00Z";

fn cmd(fake: &FakeGithub, dir: &Path, sub: &str, extra: &[&str]) -> (i32, Value) {
    let mut args = vec!["-C", dir.to_str().unwrap(), sub];
    args.extend_from_slice(extra);
    run(fake, "s1", NOW, &args)
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn files_under(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, d: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                out.push(p.strip_prefix(base).unwrap().to_string_lossy().into());
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, dir, &mut v);
    v.sort();
    v
}

fn action<'a>(v: &'a Value, path: &str) -> &'a str {
    v["data"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["path"] == path)
        .unwrap()["action"]
        .as_str()
        .unwrap()
}

fn adopted() -> (tempfile::TempDir, FakeGithub) {
    let (t, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    write(t.path(), "src/main.rs", "fn main() {}\n");
    write(t.path(), ".github/workflows/ci.yml", "name: mine\n");
    let (code, v) = cmd(&fake, t.path(), "adopt", &["--stack", "rust"]);
    assert_eq!(code, 0, "{v}");
    (t, fake)
}

#[test]
fn removes_exactly_the_adopted_files_and_empty_dirs() {
    let (t, fake) = adopted();
    let (code, v) = cmd(&fake, t.path(), "uninstall", &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        files_under(t.path()),
        [
            ".bosun/adopt.toml",
            ".github/workflows/ci.yml",
            "src/main.rs"
        ]
    );
    assert!(!t.path().join(".claude").exists());
    // The manifest stays with the labels and the registry issue, for a later --labels.
    let m = Manifest::parse(&std::fs::read_to_string(t.path().join(".bosun/adopt.toml")).unwrap())
        .unwrap();
    assert!(m.files.is_empty() && !m.labels.is_empty() && m.registry_issue.is_some());
    let (code, v) = cmd(&fake, t.path(), "uninstall", &["--labels"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        files_under(t.path()),
        [".github/workflows/ci.yml", "src/main.rs"]
    );
    assert!(fake.label_names().is_empty());
}

#[test]
fn keeps_modified_files_and_reports_gone() {
    let (t, fake) = adopted();
    write(t.path(), ".bosun/limits.toml", "# edited\n");
    std::fs::remove_file(t.path().join(".bosun/risk.toml")).unwrap();
    let (code, v) = cmd(&fake, t.path(), "uninstall", &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(action(&v, ".bosun/limits.toml"), "kept");
    assert_eq!(action(&v, ".bosun/risk.toml"), "gone");
    assert_eq!(action(&v, ".bosun/version"), "delete");
    assert_eq!(
        std::fs::read_to_string(t.path().join(".bosun/limits.toml")).unwrap(),
        "# edited\n"
    );
    assert!(t.path().join(".bosun").is_dir());
}

#[test]
fn dry_run_changes_nothing() {
    let (t, fake) = adopted();
    let before = files_under(t.path());
    let (code, v) = cmd(&fake, t.path(), "uninstall", &["--dry-run", "--labels"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(action(&v, ".bosun/version"), "delete");
    assert_eq!(files_under(t.path()), before);
    assert!(fake.label_names().iter().any(|l| l == "bosun:halt"));
}

#[test]
fn labels_deletes_created_labels_only_and_closes_registry() {
    let (t, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    fake.add_label("bosun:halt", "000000");
    let (_, a) = cmd(&fake, t.path(), "adopt", &["--stack", "rust"]);
    let n = a["data"]["registry"]["number"].as_u64().unwrap();
    let (code, v) = cmd(&fake, t.path(), "uninstall", &["--labels"]);
    assert_eq!(code, 0, "{v}");
    let names = fake.label_names();
    assert!(
        names.iter().any(|l| l == "bosun:halt"),
        "pre-existing label kept"
    );
    assert!(!names.iter().any(|l| l == "bosun:registry"));
    assert_eq!(names, ["bosun:halt"]);
    assert_eq!(fake.issue_state(n).state, "closed");
    assert_eq!(files_under(t.path()), Vec::<String>::new());
}

#[test]
fn never_touches_paths_outside_the_manifest() {
    let (t, fake) = adopted();
    let path = t.path().join(".bosun/adopt.toml");
    let mut m = Manifest::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    m.files.retain(|f| f.path != ".bosun/version");
    std::fs::write(&path, m.to_toml()).unwrap();
    cmd(&fake, t.path(), "uninstall", &[]);
    assert!(t.path().join(".bosun/version").is_file());
}

#[test]
fn no_manifest_is_an_error_with_a_fix() {
    let (t, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    let (code, v) = cmd(&fake, t.path(), "uninstall", &[]);
    assert_eq!(code, 1);
    assert_eq!(v["ok"], false);
    assert!(
        v["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("adopt-existing-repo.md")
    );
}

#[test]
fn a_manifest_path_outside_the_repo_is_never_deleted() {
    let (t, fake) = adopted();
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "victim", "precious\n");
    let rel = format!(
        "../{}/victim",
        outside.path().file_name().unwrap().to_string_lossy()
    );
    let path = t.path().join(".bosun/adopt.toml");
    let mut m = Manifest::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    m.set_file(&rel, bosun::core::adopt::fnv1a_hex(b"precious\n"));
    m.set_file("/etc/hostname", "0".into());
    std::fs::write(&path, m.to_toml()).unwrap();
    let (code, v) = cmd(&fake, t.path(), "uninstall", &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(action(&v, &rel), "kept");
    assert!(outside.path().join("victim").is_file());
    let kept = v["data"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["path"] == rel.as_str())
        .unwrap();
    assert!(!kept["fix"].as_str().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn a_symlinked_directory_is_never_followed() {
    let (t, fake) = adopted();
    let outside = tempfile::tempdir().unwrap();
    let moved = outside.path().join("gh");
    std::fs::rename(t.path().join(".github"), &moved).unwrap();
    std::os::unix::fs::symlink(&moved, t.path().join(".github")).unwrap();
    let (code, v) = cmd(&fake, t.path(), "uninstall", &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(action(&v, ".github/workflows/risk.yml"), "kept");
    assert!(moved.join("workflows/risk.yml").is_file());
    assert!(t.path().join(".github").is_symlink());
}

#[test]
fn labels_still_on_open_issues_are_kept() {
    let (t, fake) = adopted();
    fake.issue(50, "task", &["bosun:state/ready"], "");
    let (code, v) = cmd(&fake, t.path(), "uninstall", &["--labels"]);
    assert_eq!(code, 0, "{v}");
    let l = v["data"]["labels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["name"] == "bosun:state/ready")
        .unwrap();
    assert_eq!(l["action"], "in-use");
    assert!(!l["fix"].as_str().unwrap().is_empty());
    assert!(fake.label_names().iter().any(|n| n == "bosun:state/ready"));
    assert!(!fake.label_names().iter().any(|n| n == "bosun:halt"));
    let m = Manifest::parse(&std::fs::read_to_string(t.path().join(".bosun/adopt.toml")).unwrap())
        .unwrap();
    assert_eq!(m.labels, ["bosun:state/ready"]);
}

#[cfg(unix)]
#[test]
fn a_symlinked_manifest_directory_is_refused() {
    let (t, fake) = adopted();
    let outside = tempfile::tempdir().unwrap();
    let moved = outside.path().join("b");
    std::fs::rename(t.path().join(".bosun"), &moved).unwrap();
    std::os::unix::fs::symlink(&moved, t.path().join(".bosun")).unwrap();
    let (code, v) = cmd(&fake, t.path(), "uninstall", &[]);
    assert_eq!(code, 1, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
    assert!(moved.join("adopt.toml").is_file());
}

#[test]
fn only_dirs_adopt_created_are_removed() {
    let (t, fake) = (tempfile::tempdir().unwrap(), FakeGithub::start());
    std::fs::create_dir(t.path().join(".github")).unwrap();
    std::fs::create_dir(t.path().join(".claude")).unwrap();
    cmd(&fake, t.path(), "adopt", &["--stack", "rust"]);
    let (code, v) = cmd(&fake, t.path(), "uninstall", &["--labels"]);
    assert_eq!(code, 0, "{v}");
    assert!(t.path().join(".github").is_dir());
    assert!(t.path().join(".claude").is_dir());
    assert!(!t.path().join(".github/workflows").exists());
    assert!(!t.path().join(".bosun").exists());
}

#[test]
fn dry_run_with_labels_reports_in_use() {
    let (t, fake) = adopted();
    fake.issue(50, "task", &["bosun:state/ready"], "");
    let (code, v) = cmd(&fake, t.path(), "uninstall", &["--labels", "--dry-run"]);
    assert_eq!(code, 0, "{v}");
    let l = v["data"]["labels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["name"] == "bosun:state/ready")
        .unwrap();
    assert_eq!(l["action"], "in-use");
    assert!(fake.label_names().iter().any(|n| n == "bosun:halt"));
}
