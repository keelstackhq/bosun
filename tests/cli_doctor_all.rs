//! `bosun doctor`: the `gh auth token` fallback, `--remote` without a token, and `--all`.
#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use common::fake::FakeGithub;
use common::{bosun, one_object};
use serde_json::Value;

fn exec(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn doctor(
    tools: &Path,
    fake: &FakeGithub,
    token: bool,
    gh: bool,
    args: &[&str],
) -> (i32, Value, String) {
    let repo = tempfile::tempdir().unwrap();
    let mut cmd = bosun();
    cmd.env("BOSUN_GITHUB_API", fake.url())
        .env(
            "PATH",
            format!(
                "{}:{}",
                tools.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env_remove("BOSUN_SPLICE")
        .env_remove("BOSUN_HOIST");
    if !token {
        cmd.env_remove("GITHUB_TOKEN").env_remove("GH_TOKEN");
    }
    if gh {
        cmd.env_remove("BOSUN_NO_GH");
    }
    let out = cmd
        .arg("-C")
        .arg(repo.path())
        .arg("doctor")
        .args(args)
        .output()
        .unwrap();
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.code().unwrap(), one_object(&out.stdout), all)
}

#[test]
fn gh_auth_token_is_the_fallback_and_never_printed() {
    let tools = tempfile::tempdir().unwrap();
    exec(
        &tools.path().join("gh"),
        "#!/bin/sh\n[ \"$1 $2\" = \"auth token\" ] && { echo gh-secret-token; exit 0; }\nexit 1\n",
    );
    let fake = FakeGithub::start();
    let (_, v, all) = doctor(tools.path(), &fake, false, true, &[]);
    assert_eq!(v["data"]["offline"], false, "{v}");
    assert!(!all.contains("gh-secret-token"));
    let auth = fake.requests().into_iter().any(|r| {
        r.headers[0]
            .as_deref()
            .is_some_and(|a| a.contains("gh-secret-token"))
    });
    assert!(auth, "the gh token was sent to the API");
}

#[test]
fn a_missing_gh_means_offline() {
    let tools = tempfile::tempdir().unwrap();
    exec(&tools.path().join("gh"), "#!/bin/sh\nexit 1\n");
    let fake = FakeGithub::start();
    let (code, v, _) = doctor(tools.path(), &fake, false, true, &[]);
    assert_eq!(v["data"]["offline"], true, "{v}");
    assert_eq!(code, 5);
}

#[test]
fn remote_without_a_token_needs_a_human() {
    let tools = tempfile::tempdir().unwrap();
    let fake = FakeGithub::start();
    let (code, v, _) = doctor(tools.path(), &fake, false, false, &["--remote"]);
    assert_eq!(code, 5, "{v}");
    assert_eq!(v["ok"], false);
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(
        fix.contains("gh auth login") && fix.contains("GITHUB_TOKEN"),
        "{fix}"
    );
    let step = v["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == "remote checks")
        .unwrap_or_else(|| panic!("no remote step: {v}"));
    assert_eq!(step["status"], "failed", "not folded away: {v}");
}

#[test]
fn all_folds_splice_and_hoist_check_into_data_tools() {
    let tools = tempfile::tempdir().unwrap();
    exec(
        &tools.path().join("splice"),
        "#!/bin/sh\n[ \"$1\" = check ] || exit 9\necho '{\"ok\":true,\"drift\":[]}'\n",
    );
    exec(
        &tools.path().join("hoist"),
        "#!/bin/sh\n[ \"$1\" = check ] || exit 9\necho line1\necho drift found\nexit 1\n",
    );
    let fake = FakeGithub::start();
    let (_, v, _) = doctor(tools.path(), &fake, true, false, &["--all"]);
    let t = &v["data"]["tools"];
    assert_eq!(t["splice"]["installed"], true, "{v}");
    assert_eq!(t["splice"]["ok"], true);
    assert_eq!(t["splice"]["exit_code"], 0);
    assert_eq!(t["splice"]["output"]["ok"], true);
    assert_eq!(t["hoist"]["installed"], true);
    assert_eq!(t["hoist"]["ok"], false);
    assert_eq!(t["hoist"]["exit_code"], 1);
    assert!(
        t["hoist"]["output"]
            .as_str()
            .unwrap()
            .contains("drift found")
    );
    // --all alone does not run hoist as the remote step would.
    assert!(v["data"].get("hoist").is_none(), "{v}");
}

#[test]
fn all_skips_a_tool_that_is_not_installed() {
    let fake = FakeGithub::start();
    // Hide the real splice/hoist: point the overrides at names that do not exist.
    let repo = tempfile::tempdir().unwrap();
    let out = bosun()
        .env("BOSUN_GITHUB_API", fake.url())
        .env("BOSUN_SPLICE", "/nonexistent/splice")
        .env("BOSUN_HOIST", "/nonexistent/hoist")
        .arg("-C")
        .arg(repo.path())
        .args(["doctor", "--all"])
        .output()
        .unwrap();
    let v = one_object(&out.stdout);
    for t in ["splice", "hoist"] {
        let x = &v["data"]["tools"][t];
        assert_eq!(x["installed"], false, "{v}");
        assert_eq!(x["ok"], Value::Null);
        assert_eq!(x["reason"], "not installed");
    }
}
