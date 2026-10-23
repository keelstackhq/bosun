//! `bosun revert <pr>`: open a kind/mainfix PR that reverts a merged PR (spec §9). It pushes a
//! new branch only: never the default branch, never with force. bosun never merges (D13).

use std::path::Path;
use std::process::{Command, Output};

use serde_json::json;

use super::RevertArgs;
use crate::app::{App, default_branch, kill_switch, main_red};
use crate::output::{BosunError, CmdResult, Outcome};

const MAINFIX: &str = "bosun:kind/mainfix";

fn git(dir: &Path, args: &[&str]) -> Result<Output, BosunError> {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| BosunError::error(format!("cannot run git: {e}"), "install git"))
}

/// Run git and return trimmed stdout; a failure is exit 1 with the first line of stderr.
fn git_ok(dir: &Path, args: &[&str], fix: &str) -> Result<String, BosunError> {
    let out = git(dir, args)?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().next().unwrap_or("").trim();
    Err(BosunError::error(
        format!("git {} failed: {line}", args.first().unwrap_or(&"")),
        fix,
    ))
}

pub fn run(dir: Option<&Path>, args: &RevertArgs) -> CmdResult {
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    let n = args.pr;
    kill_switch(gh)?;
    let pull = gh.get_pull(n)?.ok_or_else(|| {
        BosunError::error(
            format!("PR #{n} does not exist"),
            "check the number; `bosun status` shows the main-red issue",
        )
    })?;
    let sha = match (&pull.merge_commit_sha, pull.merged) {
        (Some(sha), true) => sha.clone(),
        _ => {
            return Err(BosunError::conflict(
                format!("PR #{n} is not merged"),
                "only a merged PR can be reverted",
            ));
        }
    };
    let base = default_branch(gh)?;
    let red = main_red(gh)?;
    let branch = format!("bosun/revert-{n}");
    let wd = app.ctx.dir.as_path();
    if git(wd, &["rev-parse", "--verify", "--quiet", &branch])?
        .status
        .success()
    {
        return Err(BosunError::conflict(
            format!("the branch {branch} already exists"),
            format!("delete it with `git branch -D {branch}` or reuse its PR, then rerun"),
        ));
    }
    git(wd, &["worktree", "prune"])?;
    git_ok(
        wd,
        &["fetch", "origin", &base],
        "check the origin remote and network access",
    )?;
    // Work in a temporary detached worktree: the caller's checkout is never touched.
    let tmp = std::env::temp_dir().join(format!("bosun-revert-{n}-{}", std::process::id()));
    let tmp_s = tmp.to_string_lossy().to_string();
    let start = format!("origin/{base}");
    git_ok(
        wd,
        &["worktree", "add", "-q", "--detach", &tmp_s, &start],
        "check that the temp directory is writable",
    )?;
    let result = revert_in(&tmp, &branch, &sha);
    let _ = git(wd, &["worktree", "remove", "--force", &tmp_s]);
    let _ = std::fs::remove_dir_all(&tmp);
    let _ = git(wd, &["worktree", "prune"]);
    if let Err(e) = result {
        let _ = git(wd, &["branch", "-D", &branch]);
        return Err(e);
    }
    let warning = (pull.commits > 1
        && git_ok(wd, &["rev-list", "--parents", "-n", "1", &sha], "")
            .is_ok_and(|p| p.split_whitespace().count() == 2))
    .then(|| {
        format!(
            "PR #{n} has {} commits but its merge commit has one parent: if it was rebase-merged, \
             only the last commit is reverted",
            pull.commits
        )
    });
    let title = format!("Revert #{n}: {}", pull.title);
    let mut body = format!("Reverts #{n} ({sha}).\n\n");
    if let Some(r) = red {
        body.push_str(&format!("main-red: #{r}\n\n"));
    }
    body.push_str(&format!("<!-- bosun:revert {n} -->"));
    let created = gh.create_pull(&title, &branch, &base, &body)?;
    gh.ensure_label(MAINFIX, "b60205", "A fix for a red main")?;
    gh.add_labels(created.number, &[MAINFIX])?;
    Ok(Outcome::data(json!({
        "pr": n, "revert_pr": created.number, "branch": branch, "base": base,
        "reverted_sha": sha, "main_red": red, "warning": warning,
    })))
}

/// Create the branch in the temporary worktree, revert `sha` and push the branch.
fn revert_in(wt: &Path, branch: &str, sha: &str) -> Result<(), BosunError> {
    git_ok(
        wt,
        &["switch", "-q", "-c", branch],
        "delete the branch or pick a clean checkout",
    )?;
    let parents = git_ok(
        wt,
        &["rev-list", "--parents", "-n", "1", sha],
        "fetch the merge commit; is it on the default branch?",
    )?;
    let mut revert = vec!["revert", "--no-edit"];
    if parents.split_whitespace().count() > 2 {
        revert.extend(["-m", "1"]);
    }
    revert.push(sha);
    let out = git(wt, &revert)?;
    if !out.status.success() {
        let _ = git(wt, &["revert", "--abort"]);
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().rev().take(3).collect();
        let tail = tail.into_iter().rev().collect::<Vec<_>>().join(" | ");
        let what = if err.contains("conflict") || err.contains("CONFLICT") {
            "conflicts"
        } else {
            "fails"
        };
        return Err(BosunError::error(
            format!("git revert of {sha} {what}: {tail}"),
            "revert by hand or make a forward fix",
        ));
    }
    git_ok(
        wt,
        &["push", "-u", "origin", branch],
        "check push access to origin; bosun never pushes the default branch",
    )?;
    Ok(())
}
