//! The merge-lock on interpreter scripts through the built `bosun hook pre-tool-use` (spec §4).
mod common;

use common::bosun;
use serde_json::{Value, json};

fn hook(dir: &std::path::Path, command: &str) -> String {
    let input = json!({"session_id":"s","tool_name":"Bash","tool_input":{"command":command},
        "cwd":dir.to_str().unwrap()});
    let mut cmd = bosun();
    cmd.current_dir(dir).args(["hook", "pre-tool-use"]);
    let out = cmd.write_stdin(input.to_string()).output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{command}");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn hook_reads_the_script_an_interpreter_runs() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("m.py"),
        "import os\nos.system('gh pr merge 3')\n",
    )
    .unwrap();
    std::fs::write(tmp.path().join("ok.mjs"), "console.log('merge sort')\n").unwrap();
    let out = hook(tmp.path(), "python3 m.py");
    let v: Value = serde_json::from_str(&out).expect("one JSON object");
    let reason = v["hookSpecificOutput"]["permissionDecisionReason"]
        .as_str()
        .unwrap();
    assert!(
        reason.contains("m.py") && reason.contains("bosun pr"),
        "{reason}"
    );
    assert_eq!(hook(tmp.path(), "node ok.mjs"), "");
    assert_eq!(hook(tmp.path(), "python3 -c 'print(1)'"), "");
}
