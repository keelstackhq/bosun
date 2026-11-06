mod common;

use bosun::core::adopt::{Manifest, fnv1a_hex};
use common::fake::FakeGithub;
use common::{bosun, one_object, run};
use serde_json::Value;
use std::path::Path;

const NOW: &str = "2026-11-06T10:00:00Z";
const ALL: [&str; 13] = [
    ".bosun/bosun.toml",
    ".bosun/version",
    ".bosun/limits.toml",
    ".bosun/risk.toml",
    ".bosun/session-rules.toml",
    ".bosun/approvers.toml",
    ".bosun/fold-fragments.sh",
    ".github/workflows/bosun.yml",
    ".github/workflows/risk.yml",
    ".github/workflows/verdict.yml",
    ".github/workflows/main-open.yml",
    ".github/workflows/main-health.yml",
    ".claude/settings.json",
];

fn adopt(fake: &FakeGithub, dir: &Path, extra: &[&str]) -> (i32, Value) {
    let d = dir.to_str().unwrap();
    let mut args = vec!["-C", d, "adopt"];
    args.extend_from_slice(extra);
    run(fake, "s1", NOW, &args)
}

fn no_remote(dir: &Path, extra: &[&str]) -> Value {
    let out = bosun()
        .env_remove("GITHUB_TOKEN")
        .env_remove("BOSUN_REPO")
        .args(["-C", dir.to_str().unwrap(), "adopt", "--no-remote"])
        .args(extra)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    one_object(&out.stdout)
}

fn action<'a>(v: &'a Value, path: &str) -> &'a str {
    v["data"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["path"] == path)
        .unwrap_or_else(|| panic!("no change for {path}: {v}"))["action"]
        .as_str()
        .unwrap()
}

fn read(dir: &Path, rel: &str) -> String {
    std::fs::read_to_string(dir.join(rel)).unwrap()
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

#[test]
fn detects_stacks() {
    let fake = FakeGithub::start();
    let t = tempfile::tempdir().unwrap();
    let (_, v) = adopt(&fake, t.path(), &["--dry-run"]);
    assert_eq!(v["data"]["stacks"], serde_json::json!([]));
    write(t.path(), "Cargo.toml", "[package]\n");
    let (_, v) = adopt(&fake, t.path(), &["--dry-run"]);
    assert_eq!(v["data"]["stacks"], serde_json::json!(["rust"]));
    write(t.path(), "package.json", "{}");
    write(t.path(), "pnpm-lock.yaml", "");
    let (_, v) = adopt(&fake, t.path(), &["--dry-run"]);
    assert_eq!(v["data"]["stacks"], serde_json::json!(["rust", "ts-pnpm"]));
    let (_, v) = adopt(&fake, t.path(), &["--dry-run", "--stack", "other"]);
    assert_eq!(v["data"]["stacks"], serde_json::json!([]));
    assert!(read_opt(t.path(), ".bosun/bosun.toml").is_none());
}

fn read_opt(dir: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(rel)).ok()
}

#[test]
fn dry_run_writes_nothing_and_lists_all() {
    let fake = FakeGithub::start();
    let t = tempfile::tempdir().unwrap();
    let (code, v) = adopt(&fake, t.path(), &["--dry-run", "--stack", "rust"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["dry_run"], true);
    for p in ALL {
        assert_eq!(action(&v, p), "create", "{p}");
    }
    assert_eq!(action(&v, ".bosun/adopt.toml"), "create");
    assert_eq!(std::fs::read_dir(t.path()).unwrap().count(), 0);
    assert!(fake.label_names().is_empty());
}

#[test]
fn writes_files_labels_registry_and_manifest() {
    let fake = FakeGithub::start();
    let t = tempfile::tempdir().unwrap();
    let (code, v) = adopt(&fake, t.path(), &["--stack", "rust", "--owner", "alice"]);
    assert_eq!(code, 0, "{v}");
    for p in ALL {
        assert!(t.path().join(p).is_file(), "{p}");
    }
    assert_eq!(action(&v, "governance/owners.toml"), "create");
    assert!(read(t.path(), "governance/owners.toml").contains("github = \"alice\""));
    assert_eq!(
        read(t.path(), ".bosun/version").trim(),
        env!("CARGO_PKG_VERSION")
    );
    let cfg = read(t.path(), ".bosun/bosun.toml");
    assert!(cfg.contains("profile = \"rust\"") && cfg.contains("project = \"widget\""));
    assert!(!cfg.contains("[approvals]"));
    for p in ALL {
        let text = read(t.path(), p);
        assert!(!text.contains("{%"), "{p} has a tag left");
        assert!(
            !text.replace("${{", "").contains("{{ "),
            "{p} has a var left"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(t.path().join(".bosun/fold-fragments.sh"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111);
    }
    // No hull, splice or hoist files.
    for p in [
        ".github/workflows/splice.yml",
        "hoist.toml",
        ".github/CODEOWNERS",
        "deny.toml",
    ] {
        assert!(!t.path().join(p).exists(), "{p}");
    }
    // Remote.
    let names = fake.label_names();
    for l in [
        "bosun:halt",
        "bosun:registry",
        "bosun:state/ready",
        "bosun:state/needs-human",
    ] {
        assert!(names.iter().any(|n| n == l), "{l}");
    }
    let n = v["data"]["registry"]["number"].as_u64().unwrap();
    assert_eq!(v["data"]["registry"]["action"], "create");
    assert!(fake.labels_of(n).contains(&"bosun:registry".to_string()));
    assert!(v["data"]["owner_steps"].as_array().unwrap().len() >= 3);
    // Manifest.
    let m = Manifest::parse(&read(t.path(), ".bosun/adopt.toml")).unwrap();
    assert_eq!(m.registry_issue, Some(n));
    assert!(m.labels.contains(&"bosun:halt".to_string()));
    let f = m.files.iter().find(|f| f.path == ".bosun/version").unwrap();
    assert_eq!(
        f.hash,
        fnv1a_hex(read(t.path(), ".bosun/version").as_bytes())
    );
    assert!(m.files.iter().all(|f| f.path != ".bosun/adopt.toml"));
}

#[test]
fn second_run_is_all_unchanged() {
    let fake = FakeGithub::start();
    let t = tempfile::tempdir().unwrap();
    adopt(&fake, t.path(), &["--stack", "rust", "--owner", "alice"]);
    let first = read(t.path(), ".bosun/adopt.toml");
    let (code, v) = adopt(&fake, t.path(), &["--stack", "rust", "--owner", "alice"]);
    assert_eq!(code, 0, "{v}");
    for c in v["data"]["changes"].as_array().unwrap() {
        assert_eq!(c["action"], "unchanged", "{c}");
    }
    for l in v["data"]["labels"].as_array().unwrap() {
        assert_eq!(l["action"], "exists", "{l}");
    }
    assert_eq!(v["data"]["registry"]["action"], "exists");
    assert_eq!(read(t.path(), ".bosun/adopt.toml"), first);
}

#[test]
fn existing_differing_file_is_kept_byte_identical() {
    let fake = FakeGithub::start();
    let t = tempfile::tempdir().unwrap();
    write(t.path(), ".bosun/limits.toml", "# mine\n");
    write(t.path(), ".claude/settings.json", "{}\n");
    let (code, v) = adopt(&fake, t.path(), &["--stack", "rust"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(action(&v, ".bosun/limits.toml"), "kept");
    assert_eq!(action(&v, ".claude/settings.json"), "kept");
    let kept = v["data"]["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["path"] == ".bosun/limits.toml")
        .unwrap();
    assert!(!kept["fix"].as_str().unwrap().is_empty());
    assert_eq!(read(t.path(), ".bosun/limits.toml"), "# mine\n");
    assert_eq!(read(t.path(), ".claude/settings.json"), "{}\n");
    let m = Manifest::parse(&read(t.path(), ".bosun/adopt.toml")).unwrap();
    assert!(m.files.iter().all(|f| f.path != ".bosun/limits.toml"));
}

#[test]
fn no_remote_needs_no_token_and_repo() {
    let t = tempfile::tempdir().unwrap();
    let dir = t.path().join("my app");
    std::fs::create_dir(&dir).unwrap();
    write(&dir, "Cargo.toml", "[package]\n");
    let v = no_remote(&dir, &["--bosun-version", "0.9.1"]);
    assert_eq!(v["data"]["stacks"], serde_json::json!(["rust"]));
    assert_eq!(read(&dir, ".bosun/version").trim(), "0.9.1");
    assert!(read(&dir, ".bosun/bosun.toml").contains("project = \"my-app\""));
    let m = Manifest::parse(&read(&dir, ".bosun/adopt.toml")).unwrap();
    assert_eq!(m.bosun_version, "0.9.1");
    assert!(m.labels.is_empty() && m.registry_issue.is_none());
}

#[test]
fn existing_ci_ok_job_renders_bosun_ok_and_stays_stable() {
    let fake = FakeGithub::start();
    let t = tempfile::tempdir().unwrap();
    write(
        t.path(),
        ".github/workflows/ci.yml",
        "jobs:\n  ci-ok:\n    runs-on: ubuntu-latest\n",
    );
    adopt(&fake, t.path(), &["--stack", "rust"]);
    assert!(read(t.path(), ".github/workflows/bosun.yml").contains("bosun-ok"));
    let (_, v) = adopt(&fake, t.path(), &["--stack", "rust"]);
    assert_eq!(action(&v, ".github/workflows/bosun.yml"), "unchanged");
}

#[test]
fn preexisting_label_is_not_recorded_as_created() {
    let fake = FakeGithub::start();
    let t = tempfile::tempdir().unwrap();
    fake.add_label("bosun:halt", "000000");
    let (_, v) = adopt(&fake, t.path(), &["--stack", "rust"]);
    let halt = v["data"]["labels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["name"] == "bosun:halt")
        .unwrap();
    assert_eq!(halt["action"], "exists");
    let m = Manifest::parse(&read(t.path(), ".bosun/adopt.toml")).unwrap();
    assert!(!m.labels.contains(&"bosun:halt".to_string()));
}

#[test]
fn every_template_renders_for_each_stack_set() {
    use bosun::cli::adopt::{render_all, vars_for};
    for stacks in [
        vec!["rust"],
        vec!["ts-pnpm"],
        vec!["rust", "ts-pnpm"],
        vec![],
    ] {
        for own_ci in [false, true] {
            let stacks: Vec<String> = stacks.iter().map(|s| s.to_string()).collect();
            let vars = vars_for("app", "alice", "0.7.0", &stacks, own_ci);
            let files = render_all(&vars).unwrap_or_else(|e| panic!("{stacks:?}: {e}"));
            assert!(files.len() >= 14);
            for (path, text) in files {
                assert!(!text.contains("{%"), "{path}: tag left");
                assert!(
                    !text.contains("{{ project") && !text.contains("{{ owner"),
                    "{path}"
                );
            }
        }
    }
}
