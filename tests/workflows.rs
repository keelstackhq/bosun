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

/// The `run:` scripts of a workflow: the `run:` line and its indented block.
fn run_scripts(yaml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lines: Vec<&str> = yaml.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("run:").or_else(|| t.strip_prefix("- run:")) else {
            continue;
        };
        let indent = line.len() - t.len();
        let mut script = rest.to_string();
        for next in &lines[i + 1..] {
            let ni = next.len() - next.trim_start().len();
            if !next.trim().is_empty() && ni <= indent {
                break;
            }
            script.push('\n');
            script.push_str(next);
        }
        out.push(script);
    }
    out
}

#[test]
fn risk_judges_from_the_base_and_never_runs_pr_code() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/.github/workflows/risk.yml");
    let yaml = fs::read_to_string(path).unwrap();
    assert!(yaml.contains("\n  pull_request_target:\n"), "{yaml}");
    assert!(
        !yaml.contains("\n  pull_request:\n"),
        "pull_request runs the PR's workflow copy"
    );
    assert!(
        yaml.contains("  issue_comment:\n    types: [created]"),
        "{yaml}"
    );
    assert!(yaml.contains("contains(github.event.comment.body, '/bosun approve')"));
    // Every checkout is the base commit (or the default branch on a comment).
    let refs: Vec<&str> = yaml
        .lines()
        .filter(|l| l.trim_start().starts_with("ref:"))
        .collect();
    assert_eq!(
        refs,
        ["          ref: ${{ github.event.pull_request.base.sha || github.sha }}"],
        "{yaml}"
    );
    assert_eq!(
        yaml.matches("actions/checkout@").count(),
        1,
        "one checkout only"
    );
    for bad in [
        "head.sha",
        "head.ref",
        "head_ref",
        "refs/pull",
        "merge_commit_sha",
    ] {
        assert!(!yaml.contains(bad), "risk.yml mentions {bad}");
    }
    // No expression in a script: event data goes through env.
    let scripts = run_scripts(&yaml);
    assert!(!scripts.is_empty());
    for s in &scripts {
        assert!(!s.contains("${{"), "expression in a run script: {s}");
    }
    let perms = &yaml[yaml.find("\npermissions:\n").unwrap()..yaml.find("\njobs:").unwrap()];
    let granted: Vec<&str> = perms
        .lines()
        .skip(2)
        .filter(|l| l.starts_with("  "))
        .collect();
    assert_eq!(
        granted,
        [
            "  contents: read",
            "  issues: read",
            "  pull-requests: write",
            "  statuses: write"
        ]
    );
    assert!(
        yaml.contains("PR: ${{ github.event.pull_request.number || github.event.issue.number }}")
    );
    // Other comments must not cancel the run that judges a new head.
    assert!(yaml.contains("&& github.run_id || 'judge'"), "{yaml}");
}
