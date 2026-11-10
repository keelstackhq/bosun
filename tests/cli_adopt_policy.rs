//! `bosun adopt --policy`: writes `[approvals] policy` into the rendered `.bosun/bosun.toml`
//! (spec §8 "Autonomous policy", §11 adopt), refuses a policy the pinned release cannot parse,
//! and never silently drops the flag when the file is kept.

mod common;

use common::{bosun, one_object};
use serde_json::Value;
use std::path::Path;

fn adopt(dir: &Path, extra: &[&str]) -> (i32, Value) {
    let out = bosun()
        .env_remove("GITHUB_TOKEN")
        .env_remove("BOSUN_REPO")
        .args(["-C", dir.to_str().unwrap(), "adopt", "--no-remote"])
        .args(extra)
        .output()
        .unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

fn config(dir: &Path) -> toml::Table {
    std::fs::read_to_string(dir.join(".bosun/bosun.toml"))
        .unwrap()
        .parse()
        .unwrap()
}

fn policy_of(t: &toml::Table) -> Option<&str> {
    t.get("approvals")?.get("policy")?.as_str()
}

#[test]
fn policy_is_written_under_approvals() {
    for policy in ["owner", "owner-gated", "orchestrator", "autonomous"] {
        let t = tempfile::tempdir().unwrap();
        let (code, v) = adopt(t.path(), &["--stack", "rust", "--policy", policy]);
        assert_eq!(code, 0, "{v}");
        let c = config(t.path());
        assert_eq!(policy_of(&c), Some(policy), "{c:?}");
        // adopt always pins a version, so bootstrap_until stays off.
        assert!(c["approvals"].get("bootstrap_until").is_none(), "{c:?}");
        assert_eq!(v["data"]["policy"], policy, "{v}");
        let steps = v["data"]["owner_steps"].to_string();
        assert!(!steps.contains("by hand"), "{steps}");
    }
}

#[test]
fn without_the_flag_there_is_no_approvals_table() {
    let t = tempfile::tempdir().unwrap();
    let (code, v) = adopt(t.path(), &["--stack", "rust"]);
    assert_eq!(code, 0, "{v}");
    let text = std::fs::read_to_string(t.path().join(".bosun/bosun.toml")).unwrap();
    assert!(!text.contains("[approvals]"), "{text}");
    assert!(v["data"].get("policy").is_none_or(Value::is_null), "{v}");
}

#[test]
fn a_policy_the_pinned_release_cannot_parse_is_a_usage_error() {
    for (pin, policy, min) in [
        ("0.7.0", "autonomous", "0.8.0"),
        ("0.4.0", "owner-gated", "0.5.0"),
        ("0.4.0", "owner", "0.5.0"),
    ] {
        let t = tempfile::tempdir().unwrap();
        let (code, v) = adopt(t.path(), &["--bosun-version", pin, "--policy", policy]);
        assert_eq!(code, 2, "{pin} {policy}: {v}");
        let fix = v["error"]["fix"].as_str().unwrap();
        assert!(fix.contains(min), "{fix}");
        assert!(!t.path().join(".bosun").exists(), "wrote files");
    }
    for (pin, policy) in [("0.8.0", "autonomous"), ("0.5.0", "orchestrator")] {
        let t = tempfile::tempdir().unwrap();
        let (code, v) = adopt(t.path(), &["--bosun-version", pin, "--policy", policy]);
        assert_eq!(code, 0, "{pin} {policy}: {v}");
        assert_eq!(policy_of(&config(t.path())), Some(policy));
    }
}

#[test]
fn an_unknown_policy_is_a_usage_error() {
    let t = tempfile::tempdir().unwrap();
    let (code, v) = adopt(t.path(), &["--policy", "anyone"]);
    assert_eq!(code, 2, "{v}");
    assert!(!v["error"]["fix"].as_str().unwrap().is_empty());
}

#[test]
fn a_kept_config_gets_an_owner_step_to_set_the_policy() {
    let t = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(t.path().join(".bosun")).unwrap();
    std::fs::write(t.path().join(".bosun/bosun.toml"), "project = \"mine\"\n").unwrap();
    let (code, v) = adopt(t.path(), &["--stack", "rust", "--policy", "autonomous"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(
        std::fs::read_to_string(t.path().join(".bosun/bosun.toml")).unwrap(),
        "project = \"mine\"\n"
    );
    let steps = v["data"]["owner_steps"].to_string();
    assert!(
        steps.contains("policy = \\\"autonomous\\\"") && steps.contains("by hand"),
        "{steps}"
    );
    // Already set: nothing left to do.
    std::fs::write(
        t.path().join(".bosun/bosun.toml"),
        "project = \"mine\"\n\n[approvals]\npolicy = \"autonomous\"\n",
    )
    .unwrap();
    let (_, v) = adopt(t.path(), &["--stack", "rust", "--policy", "autonomous"]);
    assert!(
        !v["data"]["owner_steps"].to_string().contains("by hand"),
        "{v}"
    );
}

/// The channel template: an empty `policy` renders byte for byte what it did before the var,
/// and a set one adds only its lines, on the pinned path and the channel-release path alike.
#[test]
fn the_template_adds_only_the_policy_lines() {
    use bosun::cli::adopt::{render_all, vars_for};
    let config = |bosun_version: &str, policy: &str| {
        let mut vars = vars_for("app", "", "0.11.0", &["rust".to_string()], false);
        vars.set("bosun_version", bosun_version);
        vars.set("policy", policy);
        let files = render_all(&vars).unwrap();
        files
            .into_iter()
            .find(|(p, _)| *p == ".bosun/bosun.toml")
            .unwrap()
            .1
    };
    let pinned = config("0.11.0", "autonomous");
    assert_eq!(
        pinned.replace(
            "\n[approvals]\n# Who may approve (spec §8); the pinned release must know this value.\npolicy = \"autonomous\"\n",
            ""
        ),
        config("0.11.0", ""),
        "{pinned}"
    );
    let channel = config("", "autonomous");
    let t: toml::Table = channel.parse().unwrap();
    assert_eq!(t["approvals"]["policy"].as_str(), Some("autonomous"));
    assert_eq!(
        t["approvals"]["bootstrap_until"].as_str(),
        Some("2026-11-30")
    );
    assert_eq!(
        channel.replace("policy = \"autonomous\"\n", ""),
        config("", "")
    );
    assert!(!config("", "").contains("policy"));
}

fn ci_of(t: &toml::Table) -> Option<&str> {
    t.get("ci")?.get("provider")?.as_str()
}

#[test]
fn adopt_no_ci_writes_none() {
    let tmp = tempfile::tempdir().unwrap();
    let (code, v) = adopt(tmp.path(), &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(ci_of(&config(tmp.path())), Some("none"));
    assert_eq!(v["data"]["ci"], "none");
}

#[test]
fn adopt_with_workflows_writes_github() {
    let tmp = tempfile::tempdir().unwrap();
    let wf = tmp.path().join(".github/workflows");
    std::fs::create_dir_all(&wf).unwrap();
    std::fs::write(wf.join("ci.yml"), "name: ci\non: push\njobs: {}\n").unwrap();
    let (code, v) = adopt(tmp.path(), &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(ci_of(&config(tmp.path())), Some("github"));
}

#[test]
fn adopt_circleci_dir_writes_circleci() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join(".circleci")).unwrap();
    let (code, v) = adopt(tmp.path(), &[]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(ci_of(&config(tmp.path())), Some("circleci"));
}

#[test]
fn adopt_ci_flag_overrides_detection() {
    let tmp = tempfile::tempdir().unwrap();
    let (code, v) = adopt(tmp.path(), &["--ci", "github"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(ci_of(&config(tmp.path())), Some("github"));
}
