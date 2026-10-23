//! `bosun init`: add bosun's splice channel to a repo and open one setup PR (spec D16, §12 M7).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use super::InitArgs;
use crate::app::{App, default_branch};
use crate::output::{BosunError, CmdResult, Outcome};

/// Files the channel writes that a repo may already have; init keeps the repo's own.
const KEEP: [&str; 2] = [".github/workflows/splice.yml", ".claude/settings.json"];
const BRANCH: &str = "bosun/init";

const OWNER_STEPS: [&str; 6] = [
    "Fill governance/owners.toml with the owners' GitHub logins (the channel seeds it from --owner).",
    "Run `hoist apply` for branch protection, auto-merge and the required checks ci-ok, risk-check, review-verdict, main-open (hoist.toml).",
    "Create the kill-switch label: `gh label create bosun:halt --color B60205`.",
    "Add the secret SPLICE_BOT_TOKEN (a fine-grained or app token) so the splice update bot's PRs run CI.",
    "Review this setup PR; it is R-gov, so approve its head SHA (`/bosun approve <sha>`). While main has no bosun workflows, risk-check and review-verdict cannot run on it: merge it yourself, then apply the required checks.",
    "Create the Routines (janitor, curator, notify); `bosun doctor` prints their prompts.",
];

fn is_semver(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

fn git(root: &Path, args: &[&str]) -> Result<String, BosunError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| BosunError::error(format!("cannot run git: {e}"), "install git"))?;
    if !out.status.success() {
        return Err(BosunError::error(
            format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            "fix the git error above, then rerun `bosun init`",
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Run splice; its stdout is one JSON object `{ok, changes, messages}`.
fn run_splice(argv: &[String]) -> Result<Value, BosunError> {
    let bin = std::env::var("BOSUN_SPLICE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "splice".into());
    let out = Command::new(&bin)
        .args(argv)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| {
            BosunError::error(
                format!("cannot start {bin}: {e}"),
                "install splice (`npm install -g @keelstackhq/splice`) or set BOSUN_SPLICE",
            )
        })?;
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let parsed: Value = serde_json::from_str(&raw).unwrap_or(Value::String(raw));
    if out.status.success() && parsed["ok"] == true {
        return Ok(parsed);
    }
    let detail = match parsed["messages"].as_array() {
        Some(m) => m
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("; "),
        None => parsed.to_string(),
    };
    Err(BosunError::error(
        format!("splice failed (exit {:?}): {detail}", out.status.code()),
        "fix the splice error above, then `git switch -` and `git branch -D bosun/init` before rerunning",
    )
    .with_data(json!({ "splice": parsed })))
}

fn pr_body(pieces: &[String], changes: &Value, kept: &[&str]) -> String {
    let files: Vec<&str> = changes
        .as_array()
        .map(|c| c.iter().filter_map(|x| x["path"].as_str()).collect())
        .unwrap_or_default();
    let mut s = format!(
        "Sets up bosun (`bosun init`): the splice channel `bosun` with the pieces {}.\n\nFiles: `{}`.\n",
        pieces.join(", "),
        files.join("`, `")
    );
    if !kept.is_empty() {
        s += &format!("\nKept the repo's own `{}`.\n", kept.join("`, `"));
    }
    s += "\nRisk: high (R-gov). It needs the owner's approval on the head SHA.\n\nOwner steps:\n";
    for (i, step) in OWNER_STEPS.iter().enumerate() {
        s += &format!("{}. {step}\n", i + 1);
    }
    s
}

pub fn run(dir: Option<&Path>, args: &InitArgs) -> CmdResult {
    for (i, s) in args.stack.iter().enumerate() {
        if args.stack[..i].contains(s) {
            return Err(BosunError::usage(
                format!("--stack {s} is given twice"),
                "give each --stack once",
            ));
        }
    }
    if let Some(v) = args.bosun_version.as_deref().filter(|v| !is_semver(v)) {
        return Err(BosunError::usage(
            format!("--bosun-version {v:?} is not X.Y.Z"),
            "pass an exact release, such as --bosun-version 0.1.0",
        ));
    }
    if let Some(p) = args.plan.as_ref().filter(|p| !p.is_file()) {
        return Err(BosunError::usage(
            format!("plan file {} does not exist", p.display()),
            "pass --plan with the path of an existing file",
        ));
    }
    let root =
        crate::cli::gate::repo_root(&dir.map_or_else(|| PathBuf::from("."), Path::to_path_buf))?;
    if root.join(".splice/channels/bosun.toml").exists() {
        return Err(BosunError::conflict(
            "this repo already has the bosun channel",
            "the splice update bot keeps it fresh; `splice update --channel bosun` updates it by hand",
        ));
    }
    if !git(&root, &["status", "--porcelain"])?.is_empty() {
        return Err(BosunError::error(
            "the working tree is not clean",
            "commit or stash your changes, then rerun `bosun init`",
        ));
    }
    if git(
        &root,
        &[
            "rev-parse",
            "--verify",
            "-q",
            &format!("refs/heads/{BRANCH}"),
        ],
    )
    .is_ok()
    {
        return Err(BosunError::conflict(
            format!("branch {BRANCH} already exists"),
            "finish that setup, or delete the branch (`git branch -D bosun/init`) and rerun",
        ));
    }

    let version = env!("CARGO_PKG_VERSION");
    let git_ref = args
        .git_ref
        .clone()
        .unwrap_or_else(|| format!("v{version}"));
    // The repo name needs no token: BOSUN_REPO or the origin remote, else the directory name.
    let project = crate::ctx::Ctx::load(Some(&root))
        .map(|c| c.repo.name)
        .unwrap_or_else(|_| {
            root.file_name()
                .map_or("project".into(), |n| n.to_string_lossy().into_owned())
        });
    let kept: Vec<&str> = KEEP.into_iter().filter(|p| root.join(p).exists()).collect();

    let mut argv: Vec<String> = vec!["channel".into(), "add".into(), args.source.clone()];
    argv.extend([
        "--as".into(),
        "bosun".into(),
        "--ref".into(),
        git_ref.clone(),
    ]);
    for s in &args.stack {
        argv.extend(["--piece".into(), s.clone()]);
    }
    let mut data = vec![format!("project={project}")];
    data.extend(args.owner.as_ref().map(|o| format!("owner={o}")));
    // splice versions a branch or sha ref as 0.0.0+g<sha>, which is no valid pin: name one.
    let release_ref = git_ref.strip_prefix('v').is_some_and(is_semver);
    if args.bosun_version.is_some() || !release_ref {
        data.push(format!(
            "bosun_version={}",
            args.bosun_version.as_deref().unwrap_or(version)
        ));
    }
    for d in data {
        argv.extend(["--data".into(), d]);
    }
    for p in &kept {
        argv.extend(["--exclude".into(), (*p).into()]);
    }
    argv.extend(["--yes", "--json", "-C"].map(String::from));
    argv.push(root.display().to_string());

    git(&root, &["switch", "-q", "-c", BRANCH])?;
    let splice = run_splice(&argv)?;
    let changes = splice.get("changes").cloned().unwrap_or_else(|| json!([]));

    let mut plan = None;
    if let Some(src) = &args.plan {
        let rel = format!(
            "plans/{}",
            src.file_name().unwrap_or_default().to_string_lossy()
        );
        std::fs::create_dir_all(root.join("plans"))
            .and_then(|()| std::fs::copy(src, root.join(&rel)))
            .map_err(|e| {
                BosunError::error(
                    format!("cannot copy the plan to {rel}: {e}"),
                    "check that plans/ is writable",
                )
            })?;
        plan = Some(rel);
    }

    git(&root, &["add", "-A"])?;
    git(
        &root,
        &["commit", "-q", "-m", "chore: set up bosun (bosun init)"],
    )?;
    let commit = git(&root, &["rev-parse", "HEAD"])?;

    let mut pr = None;
    if !args.no_pr {
        git(&root, &["push", "-q", "-u", "origin", BRANCH])?;
        // No kill-switch check: a fresh repo has no labels yet.
        let app = App::load(Some(&root))?;
        let gh = app.gh.as_ref();
        let base = default_branch(gh)?;
        let pull = gh.create_pull(
            "chore: set up bosun",
            BRANCH,
            &base,
            &pr_body(&args.stack, &changes, &kept),
        )?;
        pr = Some(json!({"number": pull.number, "head_sha": pull.head_sha}));
    }

    Ok(Outcome::data(json!({
        "channel": "bosun", "source": args.source, "ref": git_ref, "pieces": args.stack,
        "changes": changes, "excluded": kept, "plan": plan, "branch": BRANCH,
        "commit": commit, "pr": pr, "risk": "high", "owner_steps": OWNER_STEPS,
    })))
}
