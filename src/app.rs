//! What commands share: the loaded context with a GitHub client, the kill switch, main-red,
//! state changes and the issues that hold leases.

use std::path::Path;
use std::sync::OnceLock;

use crate::config::{LIMITS_FILE, Limits};
use crate::core::glob::Glob;
use crate::core::lease::race_winner;
use crate::core::meta::{TaskMeta, parse_task_meta};
use crate::core::records::{Note, Record};
use crate::core::state::{State, labels_to_remove, state_of, transition_allowed};
use crate::ctx::Ctx;
use crate::github::{Github, Issue, IssueState, LiveGithub};
use crate::output::BosunError;
use crate::time::{Timestamp, parse_rfc3339};
use serde_json::json;

pub type Result<T> = std::result::Result<T, BosunError>;

pub const MAIN_RED_LABEL: &str = "bosun:main-red";

pub struct App {
    pub ctx: Ctx,
    pub gh: Box<dyn Github>,
}

impl App {
    /// Context from the environment and the live client for its repo.
    pub fn load(dir: Option<&Path>) -> Result<App> {
        let ctx = Ctx::load(dir)?;
        let gh = Box::new(LiveGithub::new(&ctx.repo.owner, &ctx.repo.name)?);
        Ok(App { ctx, gh })
    }
}

/// Exit 4 when the repo variable `BOSUN_HALT` is `1`. A variable that is not set (404) means
/// not halted; any other read error halts too (fail closed): a token without "Variables: read"
/// must not switch the kill switch off.
pub fn kill_switch(gh: &dyn Github) -> Result<()> {
    match gh.variable("BOSUN_HALT") {
        Ok(Some(v)) if v.trim() == "1" => Err(BosunError::halted(
            "BOSUN_HALT=1: the owner stopped all work",
            "wait for the owner to set BOSUN_HALT=0",
        )),
        Ok(_) => Ok(()),
        Err(e) => Err(BosunError::halted(
            format!("cannot read the BOSUN_HALT variable: {}", e.message),
            "give the token Variables: read, or set BOSUN_HALT=0",
        )),
    }
}

/// The repo's default branch from GitHub, looked up once per process.
pub fn default_branch(gh: &dyn Github) -> Result<String> {
    static CACHE: OnceLock<String> = OnceLock::new();
    if let Some(b) = CACHE.get() {
        return Ok(b.clone());
    }
    let info = gh.repo_info()?.ok_or_else(|| {
        BosunError::error(
            "cannot read the repository to find its default branch",
            "give the GitHub token access to the repo",
        )
    })?;
    Ok(CACHE.get_or_init(|| info.default_branch).clone())
}

/// The number of an open issue labelled `bosun:main-red`.
pub fn main_red(gh: &dyn Github) -> Result<Option<u64>> {
    Ok(gh
        .list_issues(&[MAIN_RED_LABEL], IssueState::Open)?
        .first()
        .map(|i| i.number))
}

/// Move an issue to `to`: refuse a transition the state machine forbids (exit 3), remove the
/// other state labels, add the new one.
pub fn set_state(gh: &dyn Github, number: u64, issue: &Issue, to: State) -> Result<()> {
    if let Some(from) = state_of(&issue.labels)
        && !transition_allowed(from, to)
    {
        return Err(BosunError::conflict(
            format!("#{number} cannot move from {from} to {to}"),
            format!("check `bosun status`; #{number} is {from}, which does not lead to {to}"),
        ));
    }
    gh.ensure_label(&to.label(), "ededed", "bosun task state")?;
    for label in labels_to_remove(&issue.labels, to) {
        gh.remove_label(number, &label)?;
    }
    gh.add_labels(number, &[&to.label()])
}

/// The limits that bind enforcing commands: `.bosun/limits.toml` on the default branch, or the
/// spec defaults when it is absent there. The working tree copy is not trusted: an uncommitted
/// edit must not raise a hard cap. A differing local file is mentioned on stderr.
pub fn limits_from_default(gh: &dyn Github, ctx: &Ctx) -> Result<Limits> {
    // No repo info (a bare fake or an odd token): the limits of `main` apply.
    let branch = default_branch(gh).unwrap_or_else(|_| "main".to_string());
    let limits = match gh.file_at(LIMITS_FILE, &branch)? {
        Some(text) => Limits::parse(&text, &format!("{LIMITS_FILE} on {branch}"))?,
        None => Limits::default(),
    };
    if limits != ctx.limits {
        eprintln!(
            "bosun: warning: {LIMITS_FILE} in the working tree differs from the one on the default branch ({branch}); \
             the default branch applies"
        );
    }
    Ok(limits)
}

/// The comments of an issue, oldest first.
pub fn notes(gh: &dyn Github, number: u64) -> Result<Vec<Note>> {
    Ok(gh
        .list_comments(number)?
        .into_iter()
        .map(|c| Note {
            id: c.id,
            body: c.body,
            created_at: parse_rfc3339(&c.created_at).unwrap_or(Timestamp(0)),
        })
        .collect())
}

/// The task metadata of an issue.
pub fn task_meta(issue: &Issue) -> Option<TaskMeta> {
    parse_task_meta(issue.body_text())
}

/// Open issues in a state that holds a lease, with their comments, by issue number.
pub fn active_issues(gh: &dyn Github) -> Result<Vec<(Issue, Vec<Note>)>> {
    let mut found: Vec<Issue> = Vec::new();
    for st in State::ALL.into_iter().filter(|s| s.is_active()) {
        for issue in gh.list_issues(&[&st.label()], IssueState::Open)? {
            if !found.iter().any(|i| i.number == issue.number) {
                found.push(issue);
            }
        }
    }
    found.sort_by_key(|i| i.number);
    found
        .into_iter()
        .map(|i| {
            let n = notes(gh, i.number)?;
            Ok((i, n))
        })
        .collect()
}

/// After posting a lease (comment `comment_id` on `issue`), re-read the comments of `issue` and
/// of `others` (the active issues that could hold an overlapping lease). When a live lease of
/// another claim, on `issue` or overlapping `touch`, has an earlier comment, post `lease-end`
/// (reason `lost-race`) for the new lease and return exit 3 naming the winner (spec §4).
pub fn settle_race(
    gh: &dyn Github,
    session: &str,
    issue: u64,
    comment_id: u64,
    touch: &[Glob],
    others: &[u64],
    now: Timestamp,
) -> Result<()> {
    let mut fresh = vec![(issue, notes(gh, issue)?)];
    for &n in others.iter().filter(|&&n| n != issue) {
        fresh.push((n, notes(gh, n)?));
    }
    let Some(w) = race_winner(issue, comment_id, touch, &fresh, now) else {
        return Ok(());
    };
    end_lease(gh, issue, session, "lost-race")?;
    Err(BosunError::conflict(
        format!(
            "lost the claim race for #{issue} to {} (lease on #{} until {})",
            w.session, w.issue, w.expires
        ),
        "the other lease was written first; pick another task with `bosun wave plan`, or wait for it",
    )
    .with_data(json!({"winner": {"issue": w.issue, "session": w.session,
        "expires": w.expires.to_string()}})))
}

/// Post the `lease-end` record of `session` on an issue.
pub fn end_lease(gh: &dyn Github, number: u64, session: &str, reason: &str) -> Result<u64> {
    let record = Record::LeaseEnd {
        session: session.into(),
        reason: reason.into(),
    };
    Ok(gh.add_comment(number, &record.to_line())?.id)
}
