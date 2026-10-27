//! The decrypt-lock through the built `bosun hook pre-tool-use` (spec §4).
mod common;

use common::bosun;
use serde_json::{Value, json};

#[rustfmt::skip]
const CASES: &[(&str, bool)] = &[
    ("sops secrets/prod.sops.yaml", true), ("bash -c 'sops exec-env f env'", true),
    ("age -d -i key.txt f.age", true), ("hoist secrets edit prod", true),
    ("echo $SOPS_AGE_KEY", true), ("npx sops -d f", true), ("sops --version", false),
    ("hoist secrets check --remote", false), ("git commit -m \"docs: sops decrypt\"", false),
    ("cat > d.md <<'EOF'\nrun sops -d f\nEOF", false),
];

#[test]
fn hook_denies_decrypting_and_allows_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    for (command, deny) in CASES {
        let input = json!({"session_id":"s","tool_name":"Bash","tool_input":{"command":command},
            "cwd":tmp.path().to_str().unwrap()});
        let mut cmd = bosun();
        cmd.current_dir(tmp.path()).args(["hook", "pre-tool-use"]);
        let out = cmd.write_stdin(input.to_string()).output().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(out.status.code(), Some(0), "{command}");
        if *deny {
            let v: Value = serde_json::from_str(&stdout).expect("one JSON object");
            let reason = v["hookSpecificOutput"]["permissionDecisionReason"].as_str();
            assert!(
                reason.is_some_and(|r| r.contains("hoist secrets exec")),
                "{command}"
            );
        } else {
            assert_eq!(stdout, "", "{command}");
        }
    }
}
