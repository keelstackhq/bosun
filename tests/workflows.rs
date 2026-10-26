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
    runs_the_pinned_release(&yaml);
}

/// risk.yml and verdict.yml run the release pinned in the checkout's `.bosun/version` (as
/// ci.yml's pinned-release steps) and build bosun only while the pin says `unreleased`, so a main that
/// does not compile cannot block its own mainfix.
fn runs_the_pinned_release(yaml: &str) {
    assert!(
        yaml.contains("npm install -g --ignore-scripts \"@keelstackhq/bosun@${VERSION}\""),
        "{yaml}"
    );
    assert!(
        yaml.contains("test \"$got\" = \"bosun ${VERSION}\""),
        "{yaml}"
    );
    let builds: Vec<&str> = yaml.lines().filter(|l| l.contains("cargo build")).collect();
    assert_eq!(builds.len(), 1, "{yaml}");
    let at = yaml.find(builds[0]).unwrap();
    let step = &yaml[yaml[..at].rfind("      - ").unwrap()..at];
    assert!(
        step.contains("if: steps.pin.outputs.version == ''"),
        "cargo build must only run without a pin: {step}"
    );
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
    runs_the_pinned_release(&yaml);
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
    // After the verdict, risk-check judges the PR again (spec section 8): the verdict record
    // starts no workflow. Exit 5 is the verdict; other failures fail the job.
    assert!(
        yaml.contains("PR: ${{ github.event.issue.number }}"),
        "{yaml}"
    );
    let apply = yaml.find("bosun verdict apply \"$COMMENT\"").unwrap();
    let again = yaml
        .find("./target/release/bosun risk-check \"$PR\" || rc=$?")
        .unwrap();
    assert!(apply < again, "risk-check must run after verdict apply");
    assert!(
        yaml.contains("[ \"$rc\" = 0 ] || [ \"$rc\" = 5 ]"),
        "{yaml}"
    );
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
    // Without HOIST_RELEASE_TOKEN, prepare is skipped (GITHUB_TOKEN may not open the release PR,
    // and its PR would run no CI); only whether the secret is set reaches the detect job.
    let prepare = &yaml[yaml.find("\n  prepare:\n").unwrap()..];
    let cond = prepare
        .lines()
        .find(|l| l.trim_start().starts_with("if:"))
        .unwrap();
    assert!(
        cond.contains("needs.detect.outputs.release_token == 'true'"),
        "{cond}"
    );
    assert!(yaml.contains("HAS_RELEASE_TOKEN: ${{ secrets.HOIST_RELEASE_TOKEN != '' }}"));
    assert!(yaml.contains("release_token: ${{ steps.gate.outputs.release_token }}"));
    // Event data reaches scripts through env only.
    for s in run_scripts(&yaml) {
        for bad in ["github.event", "head_ref", "github.actor", "inputs."] {
            assert!(!s.contains(bad), "run script uses {bad}: {s}");
        }
    }
}

#[test]
fn review_runs_base_code_with_the_secret_and_skips_without_review_ci() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/.github/workflows/review.yml");
    let yaml = fs::read_to_string(path).unwrap();
    let triggers = &yaml[yaml.find("\non:\n").unwrap()..yaml.find("\nconcurrency:").unwrap()];
    assert!(
        triggers.contains("\n  pull_request_target:\n"),
        "{triggers}"
    );
    for other in [
        "pull_request:",
        "pull_request_review",
        "issue_comment",
        "workflow_run",
        "push",
    ] {
        assert!(
            !triggers.contains(other),
            "review.yml must not run on {other}"
        );
    }
    // One checkout, of the base commit, without a stored token.
    let refs: Vec<&str> = yaml
        .lines()
        .filter(|l| l.trim_start().starts_with("ref:"))
        .collect();
    assert_eq!(
        refs,
        ["          ref: ${{ github.event.pull_request.base.sha }}"],
        "{yaml}"
    );
    assert_eq!(yaml.matches("actions/checkout@").count(), 1);
    assert!(yaml.contains("persist-credentials: false"));
    for bad in [
        "head.sha",
        "head.ref",
        "head_ref",
        "refs/pull",
        "merge_commit_sha",
        "/head",
        "git fetch",
    ] {
        assert!(!yaml.contains(bad), "review.yml mentions {bad}");
    }
    // Non-draft, same-repo PRs by the repo's own people: exactly these three conditions.
    let cond = &yaml[yaml.find("    if: >-\n").unwrap()..yaml.find("    environment:").unwrap()];
    let terms: Vec<&str> = cond
        .lines()
        .skip(1)
        .filter(|l| !l.trim_start().starts_with('#'))
        .map(|l| l.trim().trim_end_matches(" &&"))
        .collect();
    assert_eq!(
        terms,
        [
            "github.event.pull_request.draft == false",
            "github.event.pull_request.head.repo.full_name == github.repository",
            "contains(fromJSON('[\"OWNER\",\"MEMBER\",\"COLLABORATOR\"]'), github.event.pull_request.author_association)",
        ],
        "{cond}"
    );
    // The key lives in an environment limited to main.
    assert!(yaml.contains("\n    environment: review\n"), "{yaml}");
    assert_eq!(yaml.matches("permissions:").count(), 1);
    let perms = &yaml[yaml.find("\npermissions:\n").unwrap()..yaml.find("\njobs:").unwrap()];
    let granted: Vec<&str> = perms
        .lines()
        .skip(2)
        .filter(|l| l.starts_with("  ") && !l.trim_start().starts_with('#'))
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
    // The secret appears once, in the env of the step that runs review-ci.
    assert_eq!(yaml.matches("secrets.").count(), 1, "{yaml}");
    let step = &yaml[yaml.find("- name: bosun review-ci").unwrap()..];
    assert!(step.contains("ANTHROPIC_API_KEY: ${{ secrets.ANTHROPIC_API_KEY }}"));
    let scripts = run_scripts(&yaml);
    assert!(!scripts.is_empty());
    for s in &scripts {
        assert!(!s.contains("${{"), "expression in a run script: {s}");
    }
    // Without review-ci in the pinned release: a warning, exit 0 and no status at all, never a
    // success; with it, review-ci posts the status.
    let review = scripts
        .iter()
        .find(|s| s.contains("review-ci \"$PR\""))
        .expect("a script runs review-ci");
    let skip = &review[..review.find("fi\n").expect("an if block")];
    assert!(review.contains("review-ci --help"), "{review}");
    assert!(skip.contains("-eq 2"), "only exit 2 skips: {review}");
    assert!(skip.contains("::warning::"), "{review}");
    assert!(skip.contains("exit 0"), "{review}");
    for bad in ["statuses", "curl", "gh ", "verdict post", "verdict comment"] {
        assert!(
            !skip.contains(bad),
            "the skip path must post nothing: {bad}"
        );
    }
    runs_the_pinned_release(&yaml);
}

/// The install step has the shape of ci.yml's pinned-release steps; the job builds nothing.
fn assert_pinned_install_with_fallback(yaml: &str) {
    assert!(yaml.contains("@keelstackhq/bosun@${VERSION}"), "{yaml}");
    assert!(yaml.contains("npm install -g --ignore-scripts"));
    assert!(yaml.contains("grep -Eqx '[0-9]+\\.[0-9]+\\.[0-9]+'"));
    assert!(yaml.contains("test \"$got\" = \"bosun ${VERSION}\""));
    assert!(yaml.contains("steps.has.outputs.supported != 'true'"));
    assert!(!yaml.contains("cargo build"), "no build from main");
    // A failed install must still reach the fallback.
    assert!(yaml.contains("id: install\n        continue-on-error: true"));
    assert!(yaml.contains("INSTALL: ${{ steps.install.outcome }}"));
    assert!(yaml.contains("[ \"$INSTALL\" = success ] && [ -x target/release/bosun ]"));
    assert!(
        !yaml.contains("exit 4"),
        "the fallback exits 0; the status carries the verdict"
    );
    for line in yaml.lines().filter(|l| l.contains("::warning::")) {
        assert!(!line.contains('`'), "backtick in a shell string: {line}");
    }
    assert_eq!(
        pin_script(yaml),
        pin_script(&ci_yaml()),
        "pin step differs from ci.yml"
    );
}

fn ci_yaml() -> String {
    fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/.github/workflows/ci.yml"
    ))
    .unwrap()
}

/// The dedented `run:` script of the step with `id: pin`.
fn pin_script(yaml: &str) -> String {
    let at = yaml
        .find("        id: pin\n        run: |\n")
        .expect("a step with id: pin");
    let body = &yaml[at..];
    let body = &body[body.find("run: |\n").unwrap() + "run: |\n".len()..];
    let mut out = String::new();
    for line in body.lines() {
        if !line.is_empty() && !line.starts_with("          ") {
            break;
        }
        out.push_str(line.get(10..).unwrap_or(""));
        out.push('\n');
    }
    out
}

/// Run the pin script of each workflow under bash with a fake `.bosun/version`.
#[test]
fn pin_step_accepts_exact_versions_only() {
    if std::process::Command::new("bash")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("bash is missing: skipped");
        return;
    }
    let ci = ci_yaml();
    let mut scripts = vec![pin_script(&ci)];
    for f in ["main-open.yml", "main-health.yml"] {
        let path = format!("{}/.github/workflows/{f}", env!("CARGO_MANIFEST_DIR"));
        scripts.push(pin_script(&fs::read_to_string(path).unwrap()));
    }
    for script in &scripts {
        for (version, ok, want) in [
            ("1.2.3\n", true, "version=1.2.3"),
            ("unreleased\n", true, "version="),
            ("1.2\n", false, ""),
            ("latest\n", false, ""),
        ] {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir(dir.path().join(".bosun")).unwrap();
            fs::write(dir.path().join(".bosun/version"), version).unwrap();
            let out_file = dir.path().join("out");
            let status = std::process::Command::new("bash")
                .arg("-c")
                .arg(script)
                .current_dir(dir.path())
                .env("GITHUB_OUTPUT", &out_file)
                .output()
                .unwrap();
            assert_eq!(status.status.success(), ok, "{version:?}: {status:?}");
            if ok {
                let out = fs::read_to_string(&out_file).unwrap();
                assert_eq!(out.trim(), want, "{version:?}");
            }
        }
    }
}

#[test]
fn main_open_judges_from_the_base_and_never_runs_pr_code() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/.github/workflows/main-open.yml"
    );
    let yaml = fs::read_to_string(path).unwrap();
    assert!(yaml.contains("\n  pull_request_target:\n"), "{yaml}");
    assert!(!yaml.contains("\n  pull_request:\n"));
    let refs: Vec<&str> = yaml
        .lines()
        .filter(|l| l.trim_start().starts_with("ref:"))
        .collect();
    assert_eq!(
        refs,
        ["          ref: ${{ github.event.pull_request.base.sha }}"],
        "{yaml}"
    );
    assert_eq!(yaml.matches("actions/checkout@").count(), 1);
    assert!(yaml.contains("persist-credentials: false"));
    for bad in ["head.ref", "head_ref", "refs/pull", "merge_commit_sha"] {
        assert!(!yaml.contains(bad), "main-open.yml mentions {bad}");
    }
    // The head sha appears once, read through the API, and is used only as the status target.
    let heads: Vec<&str> = yaml.lines().filter(|l| l.contains("head.sha")).collect();
    assert_eq!(heads.len(), 1, "{heads:?}");
    assert!(heads[0].contains("sha=$(gh api \"repos/$GITHUB_REPOSITORY/pulls/$PR\""));
    let uses: Vec<&str> = yaml.lines().filter(|l| l.contains("$sha")).collect();
    assert_eq!(uses.len(), 1, "{uses:?}");
    assert!(
        uses[0].contains("repos/$GITHUB_REPOSITORY/statuses/$sha"),
        "{uses:?}"
    );
    assert!(job_ids(&yaml) == ["judge"], "{:?}", job_ids(&yaml));
    assert_pinned_install_with_fallback(&yaml);
    let scripts = run_scripts(&yaml);
    assert!(
        scripts
            .iter()
            .any(|s| s.contains("bosun gate main-open --pr \"$PR\"")),
        "{scripts:?}"
    );
    assert!(
        scripts.iter().any(|s| s.contains("context=main-open")),
        "the fallback must post the main-open status"
    );
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
            "  issues: read",
            "  pull-requests: read",
            "  statuses: write"
        ]
    );
    assert!(yaml.contains("cancel-in-progress: true"));
}

#[test]
fn main_health_reports_with_bosun_built_from_the_default_branch() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/.github/workflows/main-health.yml"
    );
    let yaml = fs::read_to_string(path).unwrap();
    let report = &yaml[yaml.find("\n  report:\n").unwrap()..];
    // The nightly job runs repo code: read-only token, not persisted.
    let nightly = &yaml[yaml.find("\n  nightly:\n").unwrap()..yaml.find("\n  report:\n").unwrap()];
    assert!(
        nightly.contains("    permissions:\n      contents: read\n    steps:"),
        "{nightly}"
    );
    assert!(nightly.contains("persist-credentials: false"), "{nightly}");
    assert!(report.contains("always()"));
    assert!(report.contains("github.event.workflow_run.event == 'push'"));
    assert!(
        report.contains("github.event.workflow_run.head_repository.full_name == github.repository")
    );
    assert!(report.contains("persist-credentials: false"));
    assert!(report.contains("ref: ${{ github.sha }}"));
    assert_pinned_install_with_fallback(report);
    assert!(
        run_scripts(report)
            .iter()
            .any(|s| s.contains("gh issue create"))
    );
    let scripts = run_scripts(report);
    assert!(
        scripts
            .iter()
            .any(|s| s.contains("bosun main-health --result \"$RESULT\" --run-url \"$RUN_URL\"")),
        "{scripts:?}"
    );
    for s in run_scripts(&yaml) {
        assert!(!s.contains("${{"), "expression in a run script: {s}");
    }
    for bad in ["head.sha", "head.ref", "head_ref", "refs/pull"] {
        assert!(!yaml.contains(bad), "main-health.yml mentions {bad}");
    }
    let perms = &yaml[yaml.find("\npermissions:\n").unwrap()..yaml.find("\njobs:").unwrap()];
    let granted: Vec<&str> = perms
        .lines()
        .skip(2)
        .filter(|l| l.starts_with("  ") && !l.trim_start().starts_with('#'))
        .collect();
    assert_eq!(
        granted,
        [
            "  contents: read",
            "  issues: write",
            "  pull-requests: read",
            "  statuses: write",
            "  checks: read"
        ]
    );
    // Weekly: a push to main already runs every gate on all of main (CI minutes).
    assert!(yaml.contains("cron: \"17 3 * * 1\""));
}

#[test]
fn splice_pins_its_version() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/.github/workflows/splice.yml");
    let yaml = fs::read_to_string(path).unwrap();
    assert!(yaml.contains("          version: \"0.2.0\"\n"), "{yaml}");
}

/// The text of ci.yml's step that starts at `from`, up to the next step.
fn step_text<'a>(yaml: &'a str, from: &str) -> &'a str {
    let i = yaml.find(from).unwrap_or_else(|| panic!("no {from:?}"));
    let rest = &yaml[i..];
    // The next step, or the comment above it, ends this one.
    let end = rest[1..]
        .match_indices("\n      ")
        .map(|(j, _)| j + 1)
        .find(|&j| rest[j + 1..].starts_with("      - ") || rest[j + 1..].starts_with("      #"))
        .map_or(rest.len(), |j| j + 1);
    &rest[..end]
}

/// CI minutes (docs/ci-minutes.md): one job does the work, `ci-ok` aggregates it; a draft fails
/// at once; an edit keeps the file checks of a passed head and reruns test-integrity, with the
/// same edit step as the channel's bosun.yml; the pin check and the gates that run no PR code
/// come before lint and test.
#[test]
fn ci_is_one_work_job_with_the_channels_edit_rule() {
    let yaml = ci_yaml();
    assert_eq!(job_ids(&yaml), ["ci", "ci-ok"]);
    let channel = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/channel/pieces/core/files/.github/workflows/bosun.yml.jinja"
    ))
    .unwrap();
    let ours = step_text(&yaml, "      - if: github.event.action == 'edited'");
    let theirs = step_text(&channel, "      - if: github.event.action == 'edited'");
    assert_eq!(
        ours,
        theirs.replace("CHECK: ${{ github.job }}", "CHECK: ci-ok"),
        "the edit step differs from the channel's"
    );
    // Every step that runs PR code comes after the trusted ones (spec §11).
    let at = |needle: &str| yaml.find(needle).unwrap_or_else(|| panic!("no {needle:?}"));
    let pin_check = at("name: the pinned release installs and reports its version");
    let gates = at("name: bosun gates");
    let npm_tests = at("name: npm launcher tests");
    assert!(pin_check < at("uses: dtolnay/rust-toolchain"));
    assert!(pin_check < at("uses: Swatinem/rust-cache"));
    assert!(
        at("run: cargo build --release --locked --bin bosun") < at("uses: Swatinem/rust-cache")
    );
    assert!(at("name: bosun of the base") < pin_check && pin_check < gates && gates < npm_tests);
    assert!(
        yaml.contains("got=\"$(\"$dir/bin/bosun\" --version)\""),
        "the pin check runs a resolved path"
    );
    assert!(yaml.contains("cache-bin: \"false\""));
    assert!(yaml.contains("          persist-credentials: false\n"));
    // Before lint, no PR code: no toolchain path, no cargo rustc swap, a checked bosun, and
    // lint and test never run after a failed gate.
    for k in [
        "  RUSTC: rustc\n",
        "  RUSTC_WRAPPER: \"\"\n",
        "  RUSTC_WORKSPACE_WRAPPER: \"\"\n",
        "  CARGO_BUILD_RUSTC_WRAPPER: \"\"\n",
        "  CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER: \"\"\n",
    ] {
        assert!(yaml.contains(k), "{k}");
    }
    // rustup's toolchain is pinned over the PR's rust-toolchain.toml, to the same channel.
    let file =
        fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/rust-toolchain.toml")).unwrap();
    let channel: toml::Table = file.parse().unwrap();
    let channel = channel["toolchain"]["channel"].as_str().unwrap();
    assert!(
        yaml.contains(&format!("  RUSTUP_TOOLCHAIN: \"{channel}\"\n")),
        "RUSTUP_TOOLCHAIN must equal rust-toolchain.toml's channel {channel}"
    );
    let guard = at("name: cargo config has no alias");
    assert!(guard < at("uses: dtolnay/rust-toolchain") && guard < at("uses: Swatinem/rust-cache"));
    assert!(at("id: binhash") < at("uses: Swatinem/rust-cache"));
    assert!(yaml.contains("BOSUN_SHA256: ${{ steps.binhash.outputs.sha256 }}"));
    assert!(yaml.contains(
        "\"$BOSUN\" gate \"$g\" $all || { echo \"::error::gate $g failed; later gates did not run\"; exit 1; }"
    ));
    assert!(
        !yaml.contains("!cancelled()"),
        "nothing runs after a failure"
    );
    let script = run_scripts(step_text(
        &yaml,
        "      - if: steps.plan.outputs.work == 'true'\n        name: cargo config has no alias",
    ))
    .remove(0);
    let script: String = script
        .lines()
        .skip(1)
        .map(|l| format!("{}\n", l.get(10..).unwrap_or("")))
        .collect();
    for (name, text, ok) in [
        (".cargo/config.toml", "[net]\nretry = 3\n", true),
        (
            ".cargo/config.toml",
            "[alias]\nfmt = [\"run\", \"--\"]\n",
            false,
        ),
        (".cargo/config", "alias.deny = \"run --\"\n", false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".cargo")).unwrap();
        fs::write(dir.path().join(name), text).unwrap();
        let passed = std::process::Command::new("bash")
            .args(["-eo", "pipefail", "-c", &script])
            .current_dir(dir.path())
            .status()
            .unwrap()
            .success();
        assert_eq!(passed, ok, "{name}: {text:?}");
    }
    // Drafts: only the plan step reads the flag; no `if` may skip a draft.
    assert_eq!(yaml.matches("pull_request.draft").count(), 1);
    assert!(yaml.contains("DRAFT: ${{ github.event.pull_request.draft || false }}"));
    assert!(yaml.contains("for g in ${ONLY:-test-integrity deps fmt lint test}; do"));
    // The plan step under the runner's shell.
    let plan = step_text(&yaml, "      - id: plan");
    let script = run_scripts(plan).remove(0);
    let script: String = script
        .lines()
        .skip(1)
        .map(|l| format!("{}\n", l.get(10..).unwrap_or("")))
        .collect();
    let run = |draft: &str, mode: &str| {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let ok = std::process::Command::new("bash")
            .args(["-eo", "pipefail", "-c", &script])
            .env("GITHUB_OUTPUT", &out)
            .env("DRAFT", draft)
            .env("MODE", mode)
            .status()
            .unwrap()
            .success();
        ok.then(|| fs::read_to_string(out).unwrap())
    };
    assert_eq!(
        run("false", "run").as_deref(),
        Some("work=true\nsetup=true\n")
    );
    assert_eq!(
        run("false", "integrity").as_deref(),
        Some("work=true\nsetup=false\nonly=test-integrity\n")
    );
    assert_eq!(run("true", "run"), None, "a draft fails");
    assert_eq!(run("true", "integrity"), None, "a draft fails");
}
