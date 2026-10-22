//! Guard for the org convention: `ci-ok` needs every other job of ci.yml, runs `if: always()`,
//! and checks each needed job's result.

use std::fs;

/// Job ids: lines indented by exactly two spaces under `jobs:` that end with `:`.
fn job_ids(yaml: &str) -> Vec<String> {
    let mut in_jobs = false;
    let mut ids = Vec::new();
    for line in yaml.lines() {
        if line == "jobs:" {
            in_jobs = true;
            continue;
        }
        if in_jobs && !line.starts_with(' ') && !line.is_empty() && !line.starts_with('#') {
            break;
        }
        if in_jobs
            && line.starts_with("  ")
            && !line.starts_with("   ")
            && !line.trim_start().starts_with('#')
            && line.trim_end().ends_with(':')
        {
            ids.push(line.trim().trim_end_matches(':').to_string());
        }
    }
    ids
}

#[test]
fn ci_ok_needs_every_job() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/.github/workflows/ci.yml");
    let yaml = fs::read_to_string(path).unwrap();
    let ids = job_ids(&yaml);
    assert!(
        ids.contains(&"ci-ok".to_string()),
        "no ci-ok job in {ids:?}"
    );
    assert!(ids.len() >= 2, "ci-ok aggregates nothing: {ids:?}");
    let ci_ok = &yaml[yaml.find("\n  ci-ok:").unwrap()..];
    let needs_line = ci_ok
        .lines()
        .find(|l| l.trim_start().starts_with("needs:"))
        .expect("ci-ok has needs:");
    assert!(
        ci_ok.contains("if: always()"),
        "ci-ok must run if: always()"
    );
    for id in ids.iter().filter(|id| *id != "ci-ok") {
        assert!(
            needs_line.contains(id.as_str()),
            "ci-ok does not need job `{id}`: {needs_line}"
        );
        let check = format!("test \"${{{{ needs.{id}.result }}}}\" = \"success\"");
        assert!(
            ci_ok.contains(&check),
            "ci-ok does not check the result of `{id}`"
        );
    }
}
