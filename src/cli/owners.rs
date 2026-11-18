//! `bosun owners set <login|org/team>...`: one owner PR that rewrites `governance/owners.toml`
//! (spec §16). Checks first (entries, clean tree on the default branch), then branch, commit,
//! push, draft PR. The file is R-gov, so the owner approves it; bosun never merges (D13).

use std::path::Path;

use serde_json::json;

use super::pin_pr::git;
use super::{OwnersArgs, OwnersCmd};
use crate::app::{App, default_branch, kill_switch};
use crate::core::risk::{owner_entries_error, parse_owners_in, render_owners};
use crate::output::{BosunError, CmdResult, Outcome};

const OWNERS: &str = "governance/owners.toml";

pub fn run(dir: Option<&Path>, a: &OwnersArgs) -> CmdResult {
    let OwnersCmd::Set(s) = &a.cmd;
    let app = App::load(dir)?;
    let org = &app.ctx.repo.owner;
    if let Some(why) = owner_entries_error(&s.entries, org) {
        return Err(BosunError::usage(
            why,
            format!("pass GitHub logins and teams `{org}/<slug>`, at least one login"),
        ));
    }
    let gh = app.gh.as_ref();
    kill_switch(gh)?;
    let root = app.ctx.dir.clone();
    let base = default_branch(gh)?;
    let on = git(&root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if on != base || !git(&root, &["status", "--porcelain"])?.is_empty() {
        return Err(BosunError::conflict(
            format!("on {on} (default branch {base}), or the tree has changes"),
            format!(
                "commit or stash the changes, run `git switch {base}` (up to date), then rerun"
            ),
        ));
    }
    let old = std::fs::read_to_string(root.join(OWNERS)).unwrap_or_default();
    let old = parse_owners_in(&old, org).unwrap_or_default();
    let text = render_owners(&s.entries, &old);
    let mut data = json!({"owners": s.entries, "changed": false, "then": []});
    if parse_owners_in(&text, org).is_ok_and(|new| new == old) {
        return Ok(Outcome::data(data));
    }
    let stamp: String = (crate::time::Timestamp::now().to_string().chars())
        .filter(char::is_ascii_digit)
        .take(14)
        .collect();
    let branch = format!("bosun/owners-{stamp}");
    let title = "chore: set the repo owners";
    std::fs::create_dir_all(root.join("governance"))
        .and_then(|()| std::fs::write(root.join(OWNERS), &text))
        .map_err(|e| {
            BosunError::error(
                format!("cannot write {OWNERS}: {e}"),
                "check the file permissions, then rerun",
            )
        })?;
    let pushed = git(&root, &["switch", "-q", "-c", &branch])
        .and_then(|_| git(&root, &["add", "--", OWNERS]))
        .and_then(|_| git(&root, &["commit", "-q", "-m", title]))
        .and_then(|_| git(&root, &["push", "-q", "-u", "origin", &branch]));
    if let Err(e) = pushed {
        // The tree was clean: back to the base, drop the branch and the edit.
        let _ = git(&root, &["switch", "-q", "-f", &base]);
        let _ = git(&root, &["branch", "-q", "-D", &branch]);
        let _ = git(&root, &["checkout", "-q", "--", OWNERS]);
        return Err(e);
    }
    let body = format!(
        "Sets the repo owners in `{OWNERS}` (`bosun owners set`). R-gov: the owner approves the head SHA.\n\nA team entry counts no member until team expansion is built (spec §16): until then the logins are the owners."
    );
    let pull = gh.create_pull(title, &branch, &base, &body, true)?;
    let n = pull.number;
    data["changed"] = json!(true);
    data["branch"] = json!(branch);
    data["pr"] = json!({"number": n, "head_sha": pull.head_sha});
    data["then"] = json!([
        format!("bosun pr --pr {n} --ready (gates, ready, auto-merge)"),
        format!("the owner approves PR #{n} at its head SHA (chat form)"),
    ]);
    Ok(Outcome::data(data))
}
