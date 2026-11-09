//! Guard for `.github/workflows/budget-report.yml`: triggers, least privilege, no event data in
//! scripts, and nothing but reports/budget/** on the reports branch.

use std::fs;

fn wf() -> String {
    fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".github/workflows/budget-report.yml"),
    )
    .unwrap()
}

/// The text of one job: from its `  <id>:` line to the next job id or the end.
fn job(yaml: &str, id: &str) -> String {
    let head = format!("  {id}:");
    let mut out = Vec::new();
    let mut inside = false;
    for line in yaml.lines() {
        let is_id = line.starts_with("  ") && !line.starts_with("   ") && line.ends_with(':');
        if line == head {
            inside = true;
        } else if inside && is_id {
            break;
        }
        if inside {
            out.push(line);
        }
    }
    assert!(!out.is_empty(), "no job {id}");
    out.join("\n")
}

#[test]
fn runs_daily_on_main_pushes_and_by_hand_one_at_a_time() {
    let y = wf();
    assert!(y.contains("  push:\n    branches: [main]"));
    assert!(y.contains("  schedule:\n    - cron:"));
    assert!(y.contains("  workflow_dispatch:"));
    assert!(y.contains("concurrency:\n  group: budget-report\n  cancel-in-progress: false"));
    for id in ["build", "report"] {
        assert!(
            job(&y, id).contains("if: github.ref == 'refs/heads/main'"),
            "{id} runs on main only"
        );
    }
}

#[test]
fn only_the_report_job_can_write_and_it_runs_no_repo_code() {
    let y = wf();
    assert!(y.starts_with("name: budget-report"));
    assert_eq!(y.matches("contents: write").count(), 1);
    assert_eq!(y.matches("pull-requests: write").count(), 1);
    let (build, report) = (job(&y, "build"), job(&y, "report"));
    assert!(build.contains("permissions:\n      contents: read"));
    assert!(!build.contains("write"));
    assert!(report.contains("contents: write") && report.contains("pull-requests: write"));
    assert!(build.contains("persist-credentials: false"));
    assert!(build.contains("cargo build --release --locked"));
    assert!(
        !report.contains("cargo "),
        "the write job never builds repo code"
    );
    assert!(report.contains("path: main\n          persist-credentials: false"));
}

#[test]
fn event_data_and_secrets_stay_out_of_scripts() {
    let y = wf();
    let mut in_run = false;
    for line in y.lines() {
        let t = line.trim_start();
        if t.starts_with("run:") {
            in_run = t.ends_with('|');
            assert!(!t.contains("${{"), "expression in a run line: {line}");
        } else if in_run && !line.starts_with("          ") {
            in_run = false;
        } else if in_run {
            assert!(!line.contains("${{"), "expression in a script: {line}");
        }
    }
    assert!(!y.contains("secrets."), "the workflow needs no secret");
    assert!(!y.contains("set -x") && !y.contains("echo \"$GH_TOKEN"));
}

#[test]
fn the_branch_holds_only_reports_and_the_pr_is_a_single_draft() {
    let y = wf();
    assert!(y.contains("BRANCH: bosun/budget-reports"));
    assert!(y.contains("git add reports/budget\n"));
    assert!(y.contains("grep -qv '^reports/budget/'"));
    assert!(y.contains("git push origin \"$BRANCH\"\n"), "no force push");
    assert!(!y.contains("--force") && !y.contains("git push -f"));
    assert!(y.contains("gh pr create --draft --base main"));
    assert!(
        y.contains("--state all"),
        "an existing PR is never made again"
    );
    assert!(!y.contains("gh pr merge") && !y.contains("--auto"));
    assert!(y.contains("--month \"$m\" --out"));
    assert!(
        y.contains("-le 3"),
        "the previous month in the first 3 days"
    );
}
