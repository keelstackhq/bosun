//! `bosun gate main-open --pr N` (spec section 9): while main is red, only a mainfix PR passes.
//! It posts the `main-open` commit status on the PR head. It does not check the kill switch: it
//! is a check, not work.

use crate::github::gate::GateWriter;
use std::collections::BTreeSet;
use std::path::Path;

use serde_json::json;

use crate::app::{App, Result, default_branch, default_ref, main_red};
use crate::config::{CONFIG_FILE, Config};
use crate::core::approve::TRUSTED_APP;
use crate::core::folders::{self, Folder};
use crate::core::meta::linked_issues;
use crate::github::{Github, Pull};
use crate::output::{BosunError, CmdResult, Outcome};

const CONTEXT: &str = "main-open";
pub const MAINFIX_LABEL: &str = "bosun:kind/mainfix";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub pass: bool,
    pub description: String,
}

/// Pass when main is open, or when the PR is a mainfix.
pub fn judge(main_red: Option<u64>, mainfix: bool) -> Verdict {
    let (pass, description) = match (main_red, mainfix) {
        (None, _) => (true, "main is open".to_string()),
        (Some(n), true) => (true, format!("main is red (#{n}); mainfix passes")),
        (Some(n), false) => (
            false,
            format!("main is red (#{n}): only mainfix PRs merge until it closes"),
        ),
    };
    Verdict { pass, description }
}

/// Main's health for one run (spec §9): the open main-red issue and, in a monorepo
/// (`[[folders]]` on the default branch), the red folders: those whose `ci-<folder>` check run
/// of GitHub Actions on the default branch head is not green. One repo: `folders` is empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MainState {
    pub red: Option<u64>,
    pub folders: Vec<Folder>,
    pub red_folders: BTreeSet<String>,
}

impl MainState {
    /// One repo, or a monorepo whose red is known only as a whole.
    pub fn whole(red: Option<u64>) -> MainState {
        MainState {
            red,
            ..MainState::default()
        }
    }
}

/// The `[[folders]]` of `.bosun/bosun.toml` on the default branch (a PR cannot change them for
/// itself); a missing or unparsable file is one repo (every PR is judged against all of main).
pub fn base_folders(gh: &dyn Github) -> Result<Vec<Folder>> {
    Ok(match gh.file_at(CONFIG_FILE, &default_ref(gh)?)? {
        Some(t) => Config::parse(&t, CONFIG_FILE)
            .map(|c| c.folder_list())
            .unwrap_or_default(),
        None => Vec::new(),
    })
}

/// Read main's state: the main-red issue and, only while main is red in a monorepo, the
/// `ci-<folder>` check runs of the default branch head (newest per name; GitHub Actions only).
pub fn main_state(gh: &dyn Github) -> Result<MainState> {
    main_state_with(gh, main_red(gh)?)
}

/// [`main_state`] for a known main-red issue `red` (`bosun main-health` has just set it).
pub fn main_state_with(gh: &dyn Github, red: Option<u64>) -> Result<MainState> {
    if red.is_none() {
        return Ok(MainState::whole(None));
    }
    let folders = base_folders(gh)?;
    if folders.is_empty() {
        return Ok(MainState::whole(red));
    }
    let runs = gh.check_runs(&default_branch(gh)?)?;
    let red_folders = folders::red_folders(&folders, |name| {
        let mine: Vec<_> = (runs.iter())
            .filter(|r| r.name == name && r.app == TRUSTED_APP)
            .collect();
        // Any red run of that name keeps the folder red; else the conclusion of a run.
        let worst = mine
            .iter()
            .find(|r| !folders::is_green_conclusion(r.conclusion.as_deref()))
            .or(mine.first());
        worst.map(|r| r.conclusion.as_deref().unwrap_or("pending"))
    });
    Ok(MainState {
        red,
        folders,
        red_folders,
    })
}

/// The folders of `main` a PR is judged against: its touched folders ([`folders::touched`];
/// a file list shorter than the PR's file count touches every folder) that are blocked (red,
/// or depending on a red folder). `None`: one repo, judge against all of main.
fn blocked_touched(gh: &dyn Github, pull: &Pull, main: &MainState) -> Result<Option<Vec<String>>> {
    if main.folders.is_empty() || main.red.is_none() {
        return Ok(None);
    }
    let files = gh.pull_files(pull.number)?;
    let paths: Vec<&str> = (files.iter())
        .flat_map(|f| std::iter::once(f.filename.as_str()).chain(f.previous_filename.as_deref()))
        .collect();
    // A list PR has no `changed_files` (0), and the files API stops at 3000: a full page or a
    // short list touches every folder.
    let touched = if (files.len() as u64) < pull.changed_files || files.len() >= 3000 {
        main.folders.iter().map(|f| f.path.clone()).collect()
    } else {
        folders::touched(&main.folders, paths)
    };
    // Main is red but no folder check is: the cause is not per-folder, so every folder is red.
    let blocked = if main.red_folders.is_empty() {
        main.folders.iter().map(|f| f.path.clone()).collect()
    } else {
        folders::blocked(&main.folders, &main.red_folders)
    };
    Ok(Some(touched.intersection(&blocked).cloned().collect()))
}

/// [`judge`] in a monorepo: `hit` is the blocked folders the PR touches (`None`: one repo).
pub fn judge_scoped(main: &MainState, mainfix: bool, hit: Option<&[String]>) -> Verdict {
    let (Some(n), Some(hit)) = (main.red, hit) else {
        return judge(main.red, mainfix);
    };
    if mainfix || !hit.is_empty() {
        let mut v = judge(main.red, mainfix);
        if !mainfix {
            v.description = format!(
                "main is red (#{n}) in {}: only mainfix PRs merge there until it closes",
                hit.join(", ")
            );
        }
        v.description = v.description.chars().take(140).collect();
        return v;
    }
    let red: Vec<&str> = main.red_folders.iter().map(String::as_str).collect();
    let description = format!(
        "main is red (#{n}) in {}; this PR touches no blocked folder",
        red.join(", ")
    );
    Verdict {
        pass: true,
        description: description.chars().take(140).collect(),
    }
}

/// A mainfix PR: the label on the PR, or the label on a linked issue that is open and is not a
/// pull request. Only labels count (they need triage rights); the task marker in a body is
/// written by its author, so it is never trusted here.
pub fn is_mainfix(gh: &dyn Github, pull: &Pull) -> Result<bool> {
    if pull.labels.iter().any(|l| l == MAINFIX_LABEL) {
        return Ok(true);
    }
    for n in linked_issues(&pull.body) {
        if let Some(issue) = gh.get_issue(n)?
            && issue.state == "open"
            && !issue.pull_request
            && issue.labels.iter().any(|l| l == MAINFIX_LABEL)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Judge one PR against `red` and post the `main-open` status on its head. Returns whether it
/// passes. `bosun main-health` re-judges every open PR with this when main turns red or green.
pub fn judge_pr(gh: &dyn Github, w: &GateWriter, pull: &Pull, main: &MainState) -> Result<bool> {
    Ok(judge_post(gh, w, pull, main)?.0.pass)
}

/// No-CI mode: judge one PR against `main` with a writer that posts nothing.
pub fn judge_local(gh: &dyn Github, pull: &Pull, main: &MainState) -> Result<Verdict> {
    Ok(judge_post(gh, &GateWriter::statuses(gh).dry(true), pull, main)?.0)
}

/// Judge and post; also returns whether it is a mainfix and the blocked folders it touches.
fn judge_post(
    gh: &dyn Github,
    w: &GateWriter,
    pull: &Pull,
    main: &MainState,
) -> Result<(Verdict, bool, Option<Vec<String>>)> {
    let mainfix = main.red.is_some() && is_mainfix(gh, pull)?;
    let hit = if mainfix {
        None
    } else {
        blocked_touched(gh, pull, main)?
    };
    let v = judge_scoped(main, mainfix, hit.as_deref());
    post(w, pull, v.clone())?;
    Ok((v, mainfix, hit))
}

fn post(w: &GateWriter, pull: &Pull, v: Verdict) -> Result<Verdict> {
    let state = if v.pass { "success" } else { "failure" };
    w.post(&pull.head_sha, CONTEXT, state, &v.description)?;
    Ok(v)
}

pub fn run(dir: Option<&Path>, pr: Option<u64>) -> CmdResult {
    let n = match pr {
        Some(n) => n,
        None => std::env::var("BOSUN_PR")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .ok_or_else(|| {
                BosunError::usage(
                    "main-open needs a PR number",
                    "pass --pr N (or set BOSUN_PR)",
                )
            })?,
    };
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    let pull = gh.get_pull(n)?.ok_or_else(|| {
        BosunError::error(
            format!("PR #{n} does not exist"),
            "check the number; `gh pr list` lists the open PRs",
        )
    })?;
    let main = main_state(gh)?;
    let red = main.red;
    let w = GateWriter::for_ctx(gh, &app.ctx);
    let (v, mainfix, hit) = judge_post(gh, &w, &pull, &main)?;
    let mut data = json!({"pr": n, "head_sha": pull.head_sha, "main_red": red,
        "mainfix": mainfix, "pass": v.pass});
    if !main.folders.is_empty() && red.is_some() {
        data["red_folders"] = json!(main.red_folders);
        data["blocked_touched"] = json!(hit.unwrap_or_default());
    }
    if w.checks_skipped() {
        data["checks_skipped"] = json!(true);
    }
    if v.pass {
        return Ok(Outcome::data(data));
    }
    let red = red.unwrap_or_default();
    Err(BosunError::halted(
        v.description,
        format!("fix main with a bosun:kind/mainfix PR, or wait for #{red} to close"),
    )
    .with_data(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_main_passes_everyone() {
        let v = judge(None, false);
        assert!(v.pass);
        assert_eq!(v.description, "main is open");
    }

    fn mono(red: &[&str]) -> MainState {
        let f = |p: &str, d: &[&str]| Folder {
            path: p.into(),
            depends_on: d.iter().map(|s| s.to_string()).collect(),
        };
        MainState {
            red: Some(7),
            folders: vec![f("bosun", &[]), f("splice", &["bosun"]), f("hoist", &[])],
            red_folders: red.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn monorepo_blocks_only_touching_a_blocked_folder() {
        let m = mono(&["bosun"]);
        let v = judge_scoped(&m, false, Some(&[]));
        assert!(v.pass, "{v:?}");
        assert!(v.description.contains("bosun"));
        let v = judge_scoped(&m, false, Some(&["splice".into()]));
        assert!(!v.pass);
        assert!(v.description.contains("#7") && v.description.contains("splice"));
        assert!(judge_scoped(&m, true, None).pass);
        // One repo: as before.
        assert_eq!(
            judge_scoped(&MainState::whole(Some(7)), false, None),
            judge(Some(7), false)
        );
        assert_eq!(
            judge_scoped(&MainState::whole(None), false, None),
            judge(None, false)
        );
    }

    #[test]
    fn red_main_passes_only_mainfix() {
        let v = judge(Some(5), true);
        assert!(v.pass);
        assert_eq!(v.description, "main is red (#5); mainfix passes");
        let v = judge(Some(5), false);
        assert!(!v.pass);
        assert!(v.description.contains("#5"));
        assert!(v.description.len() < 140);
    }
}
