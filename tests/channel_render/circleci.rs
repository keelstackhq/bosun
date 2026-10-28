//! The `circleci` piece (hull secrets-ci design §6.5): a CircleCI job runs bosun.yml's gates.

use super::*;

const JOB: &str = ".circleci/src/jobs/bosun-ok.yml";
const WORKFLOW: &str = ".circleci/src/workflows/bosun.yml";
const MODES: [&[&str]; 2] = [&[], &["circleci"]];
const STACKS: [&[&str]; 3] = [&["rust"], &["ts-pnpm"], &["rust", "ts-pnpm"]];

fn mode(stack: &[&str], m: &[&str], own_ci: &str) -> Vec<(String, String)> {
    variant(&[stack, m].concat(), own_ci)
}

/// A rendered file, `""` when absent or rendered empty (splice then writes nothing).
fn text<'a>(files: &'a [(String, String)], name: &str) -> &'a str {
    files
        .iter()
        .find(|(n, _)| n == name)
        .map_or("", |(_, t)| t.as_str())
}

#[test]
fn the_circleci_job_carries_the_required_checks_name_on_every_branch() {
    for own_ci in ["", "1"] {
        let files = mode(&["rust"], &["circleci"], own_ci);
        let Some(wf) = yaml_json(text(&files, WORKFLOW)) else {
            return;
        };
        let job = if own_ci.is_empty() {
            serde_json::json!({"bosun-ok": {"name": "ci-ok"}})
        } else {
            serde_json::json!("bosun-ok")
        };
        // No branch, path or `when` filter: PR branches, gh-readonly-queue/* and scheduled runs.
        assert_eq!(wf, serde_json::json!({"jobs": [job]}), "own_ci={own_ci:?}");
        assert!(yaml_json(text(&files, JOB)).unwrap()["steps"].is_array());
        for name in [JOB, WORKFLOW] {
            assert!(!text(&files, name).contains("paths:"), "{name}");
        }
    }
    assert!(
        !text(&variant(&["rust"], ""), JOB).contains("steps"),
        "no CircleCI without the piece"
    );
}

#[test]
fn governance_workflows_are_the_same_in_every_mode() {
    for stack in STACKS {
        for own_ci in ["", "1"] {
            let github = variant(stack, own_ci);
            for m in MODES {
                let files = mode(stack, m, own_ci);
                for wf in ["risk", "verdict", "main-open", "splice"] {
                    let name = format!(".github/workflows/{wf}.yml");
                    assert!(!text(&files, &name).is_empty(), "{name}");
                    assert_eq!(text(&files, &name), text(&github, &name), "{name} {m:?}");
                }
            }
        }
    }
}

/// Every run of gate names in the `GATES:` lines of a file, as space-joined lists.
fn gate_lists(text: &str) -> Vec<String> {
    let mut lists = Vec::new();
    for line in text.lines().filter(|l| l.contains("GATES:")) {
        let mut cur: Vec<&str> = Vec::new();
        for w in line.split_whitespace().map(|w| w.trim_matches(['\'', '"'])) {
            if ALL_GATES.contains(&w) {
                cur.push(w);
            } else if !cur.is_empty() {
                lists.push(cur.join(" "));
                cur.clear();
            }
        }
        if !cur.is_empty() {
            lists.push(cur.join(" "));
        }
    }
    lists.sort();
    lists.dedup();
    lists
}

#[test]
fn the_circleci_job_runs_the_gates_of_bosun_yml() {
    for stack in STACKS {
        for own_ci in ["", "1"] {
            let files = mode(stack, &["circleci"], own_ci);
            let (gh, ci) = (
                gate_lists(text(&files, BOSUN_YML)),
                gate_lists(text(&files, JOB)),
            );
            let mut want = vec![ALL_GATES.join(" ")];
            if !own_ci.is_empty() {
                want.insert(0, "test-integrity".to_string());
            }
            assert_eq!(gh, want, "{stack:?} own_ci={own_ci:?}");
            assert_eq!(ci, gh, "{stack:?} own_ci={own_ci:?}");
            let job = text(&files, JOB);
            assert!(job.contains("for g in $GATES; do"), "{job}");
            assert!(job.contains(r#""$bosun" gate "$g""#), "{job}");
        }
    }
}

#[test]
fn the_default_branch_is_fetched_before_the_pin_and_every_gate() {
    for stack in STACKS {
        for own_ci in ["", "1"] {
            let job = text(&mode(stack, &["circleci"], own_ci), JOB).to_string();
            let fetch = job
                .find(r#"git fetch --no-tags origin "+refs/heads/$DEFAULT_BRANCH:refs/remotes/origin/$DEFAULT_BRANCH""#)
                .unwrap_or_else(|| panic!("no fetch of the default branch: {job}"));
            assert!(job.contains("DEFAULT_BRANCH: main"), "{job}");
            for later in [
                "git merge-base HEAD",
                ".bosun/version",
                "gate \"$g\"",
                "test-integrity",
            ] {
                let at = job.find(later).unwrap_or_else(|| panic!("no {later}"));
                assert!(fetch < at, "{later} runs before the fetch");
            }
        }
    }
}

#[test]
fn the_circleci_job_runs_a_pinned_bosun_checked_by_sha256() {
    for stack in STACKS {
        let job = text(&mode(stack, &["circleci"], ""), JOB).to_string();
        for want in [
            r#"npm install -g --ignore-scripts "@keelstackhq/bosun@${v}""#,
            r"grep -Eqx '[0-9]+\.[0-9]+\.[0-9]+'",
            r#"test "$got" = "bosun ${v}""#,
            r#"echo "$BOSUN_SHA256  $bosun" | sha256sum -c --quiet"#,
            "RUSTC_WRAPPER: \"\"",
        ] {
            assert!(job.contains(want), "{stack:?}: no {want}");
        }
        let check = job.find("sha256sum -c --quiet").unwrap();
        assert!(check < job.find(r#""$bosun" gate"#).unwrap());
        // Every download is checked against a pinned SHA-256 on the next line.
        let lines: Vec<&str> = job.lines().collect();
        for (i, l) in lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.contains("curl "))
        {
            assert!(lines[i + 1].contains("| sha256sum -c - || exit 1"), "{l}");
        }
        // The image is pinned by digest.
        assert!(job.contains("image: cimg/node:24.22.1@sha256:"), "{job}");
    }
}
