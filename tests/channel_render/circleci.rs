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

/// A step of the rendered job: its script and its `environment`.
fn job_step(stack: &[&str], own_ci: &str, name: &str) -> Option<(String, Vec<(String, String)>)> {
    let j = yaml_json(text(&mode(stack, &["circleci"], own_ci), JOB))?;
    let steps = j["steps"].as_array().unwrap();
    let run = steps.iter().find(|s| s["run"]["name"] == name).unwrap()["run"].clone();
    let env = run["environment"].as_object().map_or(Vec::new(), |m| {
        m.iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
            .collect()
    });
    Some((run["command"].as_str().unwrap().to_string(), env))
}

/// A repo whose `origin/main` is commit `main` and HEAD is `head`, both as `git` commands; a
/// fake bosun in `$HOME/.bosun-ci` logs `gate <args>` and fails the gate named in `FAIL`.
fn ci_repo(main: &str, head: &str) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    fs::create_dir_all(d.join("home/.bosun-ci")).unwrap();
    fs::create_dir_all(d.join("repo")).unwrap();
    let bin = d.join("home/.bosun-ci/bosun");
    let fake = "#!/bin/sh\necho \"$* $(cat package.json 2>/dev/null)\" >> \"$HOME/log\"\n[ \"$2\" != \"$FAIL\" ]\n";
    fs::write(&bin, fake).unwrap();
    let sum = git_in(
        d,
        "chmod +x home/.bosun-ci/bosun && sha256sum home/.bosun-ci/bosun | cut -d' ' -f1",
    );
    let r = d.join("repo");
    git_in(
        &r,
        "git init -q -b main && git config user.email a@b.c && git config user.name n",
    );
    git_in(&r, main);
    git_in(&r, "git update-ref refs/remotes/origin/main HEAD");
    git_in(&r, head);
    (dir, sum)
}

/// Runs a job step in the repo; `(exit ok, the fake bosun's log)`.
fn run_step(
    dir: &Path,
    sum: &str,
    step: &(String, Vec<(String, String)>),
    env: &[(&str, &str)],
) -> (bool, String) {
    let base = git_in(&dir.join("repo"), "git rev-parse origin/main");
    let home = dir.join("home");
    let mut all: Vec<(&str, &str)> =
        vec![("HOME", home.to_str().unwrap()), ("DEFAULT_BRANCH", "main")];
    all.extend([
        ("BOSUN_SHA256", sum),
        ("BOSUN_BASE", base.as_str()),
        ("FAIL", ""),
    ]);
    all.extend(step.1.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    all.extend_from_slice(env);
    let ok = bash_in(&dir.join("repo"), &step.0, &all);
    (ok, fs::read_to_string(home.join("log")).unwrap_or_default())
}

const COMMIT: &str = "git add -A && git commit -qm c";

#[test]
fn the_circleci_gates_run_what_the_branch_needs_and_stop_at_a_failure() {
    let ahead = "git checkout -qb pr && echo x > f && git add -A && git commit -qm pr";
    let five = |all: &str| ALL_GATES.map(|g| format!("gate {g}{all} \n")).concat();
    for own_ci in ["", "1"] {
        let Some(step) = job_step(&["rust"], own_ci, "bosun gates") else {
            return;
        };
        let (d, sum) = ci_repo(&format!("echo a > a && {COMMIT}"), ahead);
        let (_, log) = run_step(d.path(), &sum, &step, &[("CIRCLE_BRANCH", "feat/x")]);
        let pr = if own_ci.is_empty() {
            five("")
        } else {
            "gate test-integrity \n".into()
        };
        assert_eq!(log, pr, "own_ci={own_ci:?}");
        for branch in ["main", "gh-readonly-queue/main/pr-1-ab"] {
            let (d, sum) = ci_repo(&format!("echo a > a && {COMMIT}"), ahead);
            let (ok, log) = run_step(d.path(), &sum, &step, &[("CIRCLE_BRANCH", branch)]);
            assert!(ok && log == five(""), "{branch}: {log}");
        }
        // HEAD is the merge-base (a fresh branch): every gate, on everything.
        let (d, sum) = ci_repo(&format!("echo a > a && {COMMIT}"), "git checkout -qb fresh");
        let (_, log) = run_step(d.path(), &sum, &step, &[("CIRCLE_BRANCH", "fresh")]);
        assert_eq!(log, five(" --all"));
        let (d, sum) = ci_repo(&format!("echo a > a && {COMMIT}"), ahead);
        let (ok, log) = run_step(
            d.path(),
            &sum,
            &step,
            &[("CIRCLE_BRANCH", "main"), ("FAIL", "deps")],
        );
        assert!(!ok, "a failed gate fails the job");
        assert_eq!(
            log, "gate test-integrity \ngate deps \n",
            "no gate after a failure"
        );
    }
}

#[test]
fn the_circleci_gate_config_and_pin_come_from_the_default_branch_tip() {
    let job = text(&mode(&["rust"], &["circleci"], ""), JOB).to_string();
    assert!(
        job.contains(r#"base="$(git rev-parse "origin/$DEFAULT_BRANCH")""#),
        "{job}"
    );
    let Some(step) = job_step(&["rust"], "", "the base's config for the gates") else {
        return;
    };
    let prof = ".bosun/profiles/rust.toml";
    // The PR branches off an old main commit (before bosun, too); main's tip pins the config.
    for old in [
        "echo a > a",
        "mkdir -p .bosun/profiles && echo old > .bosun/profiles/rust.toml",
    ] {
        let main = format!(
            "{old} && {COMMIT} && git tag old && rm -rf .bosun && mkdir -p .bosun/profiles && echo tip > {prof} && echo 1.2.3 > .bosun/version && {COMMIT}"
        );
        let pr = format!(
            "git checkout -qb pr old && mkdir -p .bosun/profiles && echo pr > {prof} && echo 1.2.3 > .bosun/version && {COMMIT}"
        );
        let (d, sum) = ci_repo(&main, &pr);
        assert!(run_step(d.path(), &sum, &step, &[]).0);
        assert_eq!(
            fs::read_to_string(d.path().join("repo").join(prof)).unwrap(),
            "tip\n"
        );
    }
}

#[test]
fn circleci_deps_audits_with_the_bases_manifests_and_the_rest_sees_the_prs() {
    let Some(step) = job_step(&["ts-pnpm"], "", "bosun gates") else {
        return;
    };
    let main = format!("echo base > package.json && echo base > pnpm-workspace.yaml && {COMMIT}");
    let pr = format!(
        "git checkout -qb pr && echo pr > package.json && rm pnpm-workspace.yaml && {COMMIT}"
    );
    let (d, sum) = ci_repo(&main, &pr);
    let (ok, log) = run_step(d.path(), &sum, &step, &[("CIRCLE_BRANCH", "pr")]);
    assert!(ok, "{log}");
    let want = "gate test-integrity pr\ngate deps base\ngate fmt pr\ngate lint pr\ngate test pr\n";
    assert_eq!(log, want);
    assert!(
        !d.path().join("repo/pnpm-workspace.yaml").exists(),
        "a base-only copy goes"
    );
    // A PR that links a stack directory outside the checkout stops deps; nothing outside goes.
    let main = format!("mkdir web && echo base > web/package.json && {COMMIT}");
    let out = "mkdir -p ../out && echo keep > ../out/package.json";
    let pr = format!("git checkout -qb pr && rm -r web && {out} && ln -s ../out web && {COMMIT}");
    let (d, sum) = ci_repo(&main, &pr);
    let (ok, log) = run_step(d.path(), &sum, &step, &[("CIRCLE_BRANCH", "pr")]);
    assert!(!ok, "{log}");
    assert_eq!(log, "gate test-integrity \n", "deps never ran");
    assert!(d.path().join("out/package.json").exists());
}

#[test]
fn the_circleci_toolchain_is_validated_and_pinned_for_later_steps() {
    let Some((script, _)) = job_step(&["rust"], "", "rust toolchain") else {
        return;
    };
    // `$HOME/project` is the checkout; a fake rustup logs its args and directory. Rows: files
    // (`path=text`, `|`-separated), where the step runs, and the first rustup call and channel
    // (none: refused). A parent's file is found; rustup installs what it names (targets too)
    // from its directory; rust-toolchain wins over rust-toolchain.toml.
    let toml = "[toolchain]\nchannel = \"1.95.0\"\ntargets = [\"wasm32-unknown-unknown\"]\n";
    let rows = [
        (
            String::new(),
            "sub",
            Some((
                "toolchain install 1.97 --profile minimal -c rustfmt,clippy @sub",
                "1.97",
            )),
        ),
        (
            format!("rust-toolchain.toml={toml}"),
            "sub",
            Some(("toolchain install @project", "1.95.0")),
        ),
        (
            format!("sub/rust-toolchain.toml={toml}|sub/rust-toolchain=nightly-2026-01-01\n"),
            "sub",
            Some(("toolchain install @sub", "nightly-2026-01-01")),
        ),
        (
            "rust-toolchain.toml=[toolchain]\npath = \"/opt/evil\"\n".into(),
            ".",
            None,
        ),
        (
            "rust-toolchain.toml=[toolchain]\nchannel = \"../x\"\n".into(),
            ".",
            None,
        ),
        (
            "rust-toolchain.toml=channel = \"1.95.0\"\n".into(),
            ".",
            None,
        ),
    ];
    for (files, cwd, want) in rows {
        let dir = tempfile::tempdir().unwrap();
        let (home, project) = (dir.path(), dir.path().join("project"));
        fs::create_dir_all(project.join("sub")).unwrap();
        for (f, t) in files.split('|').filter_map(|e| e.split_once('=')) {
            fs::write(project.join(f), t).unwrap();
        }
        fs::write(
            home.join("rustup"),
            "#!/bin/sh\necho \"$* @${PWD##*/}\" >> \"$HOME/log\"\n",
        )
        .unwrap();
        git_in(home, "chmod +x rustup");
        let path = format!("{}:{}", home.display(), std::env::var("PATH").unwrap());
        let (h, e) = (
            home.to_str().unwrap(),
            home.join("env").display().to_string(),
        );
        let env = [
            ("HOME", h),
            ("PATH", path.as_str()),
            ("BASH_ENV", e.as_str()),
        ];
        let ok = bash_in(&project.join(cwd), &script, &env);
        let log = fs::read_to_string(home.join("log")).unwrap_or_default();
        let Some((install, ch)) = want else {
            assert!(!ok && log.is_empty(), "{files}: {log}");
            continue;
        };
        assert!(ok && log.lines().next() == Some(install), "{files}: {log}");
        let pinned = fs::read_to_string(&e).unwrap();
        assert_eq!(pinned, format!("export RUSTUP_TOOLCHAIN={ch}\n"));
        let components = format!("component add --toolchain {ch} rustfmt clippy");
        assert!(ch == "1.97" || log.contains(&components), "{log}");
    }
    let Some((_, env)) = job_step(&["rust"], "", "bosun gates") else {
        return;
    };
    for k in ["BOSUN_INTEGRITY_ALLOW", "BOSUN_PR_BODY"] {
        assert!(
            env.contains(&(k.to_string(), String::new())),
            "{k} is cleared: {env:?}"
        );
    }
}
