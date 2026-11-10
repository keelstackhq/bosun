//! `bosun schema --format ts`: TypeScript declarations of the contract.

mod common;

use bosun::schema;
use common::{bosun, one_object};
use serde_json::Value;

fn schema_cmd(args: &[&str]) -> (i32, Value) {
    let out = bosun().arg("schema").args(args).output().unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

#[test]
fn ts_declares_every_command() {
    let (code, v) = schema_cmd(&["--format", "ts"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["format"], "ts");
    let ts = v["data"]["ts"].as_str().unwrap();
    assert!(ts.starts_with("// bosun's output contract"));
    assert!(ts.contains("declare namespace bosun {"));
    assert!(ts.contains("type Envelope<C extends string, D>"));
    assert!(ts.contains(
        "function verdictPost(...args: string[]): Promise<Envelope<\"verdict post\", VerdictPostData>>;"
    ));
    assert!(ts.contains("function riskCheck(...args: string[])"));
    assert!(ts.contains("type HeartbeatData = { leases: number[]; registry: boolean };"));
    assert_eq!(ts.matches("  function ").count(), schema::names().count());
}

#[test]
fn ts_for_one_command() {
    let (code, v) = schema_cmd(&["claim", "--format", "ts"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["command"], "claim");
    let ts = v["data"]["ts"].as_str().unwrap();
    assert!(ts.contains("lease: { expires: string; session: string; touch: string[] }"));
    assert!(ts.contains("added?: string[]"));
    assert_eq!(ts.matches("  function ").count(), 1);
}

#[test]
fn a_group_names_its_subcommands() {
    let (code, v) = schema_cmd(&["verdict"]);
    assert_eq!(code, 2);
    let fix = v["error"]["fix"].as_str().unwrap();
    assert!(fix.contains("<post|comment|apply>"), "{fix}");
}

/// When `tsc` is on PATH (the dev container has it), the declarations compile.
#[test]
fn ts_compiles_when_tsc_is_there() {
    let Ok(tsc) = which_tsc() else { return };
    let (_, v) = schema_cmd(&["--format", "ts"]);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bosun.d.ts");
    std::fs::write(&file, v["data"]["ts"].as_str().unwrap()).unwrap();
    let out = std::process::Command::new(tsc)
        .args(["--noEmit", "--strict"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

fn which_tsc() -> Result<std::path::PathBuf, ()> {
    let path = std::env::var_os("PATH").ok_or(())?;
    std::env::split_paths(&path)
        .map(|d| d.join("tsc"))
        .find(|p| p.is_file())
        .ok_or(())
}
