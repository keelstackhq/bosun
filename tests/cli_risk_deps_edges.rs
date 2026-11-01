//! R-deps counts new outside code, not new edges to pins that already exist (spec §8).
mod common;

use common::fake::FakeGithub;
use common::run;
use serde_json::Value;

const NOW: &str = "2026-10-22T10:00:00Z";
const BASE: &str = "base0000";
const HEAD: &str = "sha1";

/// PR 1 judged by the dependency rule only.
fn pr() -> FakeGithub {
    let fake = FakeGithub::start();
    fake.pull(1, "change", "feature", "");
    fake.content(
        "governance/owners.toml",
        BASE,
        "[[owner]]\ngithub = \"ada\"\n",
    );
    fake.content(
        ".bosun/risk.toml",
        BASE,
        "[[rule]]\nid = \"R-deps\"\nwhy = \"d\"\nnew_dependency = true\n",
    );
    fake
}

fn changed(fake: &FakeGithub, path: &str, base: &str, head: &str) {
    fake.pull_file(1, path, "modified", 1, 0, Some("@@ -0,0 +1,1 @@\n+x"));
    fake.content(path, BASE, base);
    fake.content(path, HEAD, head);
}

fn check(fake: &FakeGithub) -> (i32, Value) {
    run(fake, "session_test", NOW, &["risk-check", "1"])
}

#[test]
fn a_workspace_edge_to_an_existing_pin_is_normal() {
    let fake = pr();
    changed(
        &fake,
        "crates/app/Cargo.toml",
        "[package]\nname = \"app\"\n[dependencies]\n",
        "[package]\nname = \"app\"\n[dependencies]\ntokio.workspace = true\n\
         core = { path = \"../core\" }\n",
    );
    let (code, v) = check(&fake);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["level"], "normal");
}

#[test]
fn a_new_root_pin_and_a_new_catalog_entry_match_r_deps() {
    let fake = pr();
    changed(
        &fake,
        "Cargo.toml",
        "[workspace.dependencies]\ntokio = \"1\"\n",
        "[workspace.dependencies]\ntokio = \"1\"\nevil = \"0.1\"\n",
    );
    changed(
        &fake,
        "pnpm-workspace.yaml",
        "catalog:\n  react: ^18\n",
        "catalog:\n  react: ^18\n  left-pad: 1.3.0\n",
    );
    let (code, v) = check(&fake);
    assert_eq!(code, 5, "{v}");
    let evidence = v["data"]["matches"][0]["evidence"].to_string();
    assert!(evidence.contains("new dependency evil"), "{evidence}");
    assert!(evidence.contains("new dependency left-pad"), "{evidence}");
}
