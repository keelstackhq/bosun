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
    assert!(
        yaml.contains("persist-credentials: false"),
        "the token must not stay in .git"
    );
    assert_eq!(
        yaml.matches("permissions:").count(),
        1,
        "no job-level permissions"
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

#[test]
fn verdict_runs_main_code_on_verdict_comments_only() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/.github/workflows/verdict.yml");
    let yaml = fs::read_to_string(path).unwrap();
    let triggers = &yaml[yaml.find("\non:\n").unwrap()..yaml.find("\npermissions:").unwrap()];
    assert!(
        triggers.contains("  issue_comment:\n    types: [created]"),
        "{triggers}"
    );
    for other in ["pull_request", "workflow_run", "push"] {
        assert!(
            !triggers.contains(other),
            "verdict.yml must not run on {other}"
        );
    }
    assert!(yaml.contains("contains(github.event.comment.body, '<!-- bosun:verdict -->')"));
    let refs: Vec<&str> = yaml
        .lines()
        .filter(|l| l.trim_start().starts_with("ref:"))
        .collect();
    assert_eq!(refs, ["          ref: ${{ github.sha }}"], "{yaml}");
    assert_eq!(yaml.matches("actions/checkout@").count(), 1);
    assert!(yaml.contains("persist-credentials: false"));
    for bad in [
        "head.sha",
        "head.ref",
        "head_ref",
        "refs/pull",
        "merge_commit_sha",
    ] {
        assert!(!yaml.contains(bad), "verdict.yml mentions {bad}");
    }
    let scripts = run_scripts(&yaml);
    assert!(!scripts.is_empty());
    for s in &scripts {
        assert!(!s.contains("${{"), "expression in a run script: {s}");
    }
    assert_eq!(yaml.matches("permissions:").count(), 1);
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
            "  issues: write",
            "  pull-requests: write",
            "  statuses: write"
        ]
    );
    assert!(yaml.contains("COMMENT: ${{ github.event.comment.id }}"));
    // Other comments must not replace a pending verdict run.
    assert!(yaml.contains("&& github.run_id || 'apply'"), "{yaml}");
    assert!(yaml.contains("cancel-in-progress: false"));
}

#[test]
fn release_runs_only_on_main_pushes_and_never_puts_event_data_in_a_script() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/.github/workflows/release.yml");
    let yaml = fs::read_to_string(path).unwrap();
    // Only a push to main or a manual run: no pull request or comment reaches publish rights.
    let triggers = &yaml[yaml.find("\non:\n").unwrap()..yaml.find("\npermissions:").unwrap()];
    assert_eq!(
        triggers.trim(),
        "on:\n  push:\n    branches: [main]\n  workflow_dispatch:",
        "{triggers}"
    );
    // Read-only by default; only the publish and prepare jobs widen it.
    assert!(
        yaml.contains("\npermissions:\n  contents: read\n\n"),
        "{yaml}"
    );
    let jobs = &yaml[yaml.find("\njobs:\n").unwrap()..];
    let mut job = String::new();
    for line in jobs.lines() {
        let line = line.split(" #").next().unwrap_or(line).trim_end();
        if line.starts_with("  ") && !line.starts_with("   ") && line.ends_with(':') {
            job = line.trim().trim_end_matches(':').to_string();
        }
        // `contents: write`, `permissions: write-all` and `{ contents: write }` all grant.
        let tight: String = line.split_whitespace().collect();
        let grants = tight.ends_with(":write")
            || tight.contains("write-all")
            || tight.contains(":write,")
            || tight.contains(":write}");
        if grants {
            assert!(
                job == "publish" || job == "prepare",
                "job `{job}` grants {}",
                line.trim()
            );
        }
    }
    // Event data reaches scripts through env only.
    for s in run_scripts(&yaml) {
        for bad in ["github.event", "head_ref", "github.actor", "inputs."] {
            assert!(!s.contains(bad), "run script uses {bad}: {s}");
        }
    }
}
