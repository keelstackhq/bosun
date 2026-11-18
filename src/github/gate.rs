//! The one writer of the gate contexts `risk-check`, `review-verdict` and `main-open` (spec §15):
//! a commit status, a check run of the gate App, or both, by `[checks] mode`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::OnceLock;

use super::{CheckRun, Comment, GateRun, Github, LiveGithub, Result, RunState, Status};
use crate::config::{CheckMode, Checks};
use crate::core::approve::TRUSTED_CREATOR;
use crate::core::risk::VERDICT_AUTHOR;
use crate::output::BosunError;

/// Env var holding the gate App's installation token (never argv, never printed).
pub const APP_TOKEN_ENV: &str = "BOSUN_APP_TOKEN";

pub struct GateWriter<'a> {
    gh: &'a dyn Github,
    app: Option<Box<dyn Github>>,
    mode: CheckMode,
    /// `(sha, context)` -> id of the `in_progress` run this process created, for the verdict.
    pending: RefCell<HashMap<(String, String), u64>>,
    skipped: Cell<bool>,
    dry: bool,
}

impl<'a> GateWriter<'a> {
    /// `gh` posts the statuses; `app` (the App token's client) the check runs.
    pub fn new(gh: &'a dyn Github, app: Option<Box<dyn Github>>, mode: CheckMode) -> Self {
        GateWriter {
            gh,
            app,
            mode,
            pending: RefCell::new(HashMap::new()),
            skipped: Cell::new(false),
            dry: false,
        }
    }

    /// Whether this writer writes nothing.
    pub fn is_dry(&self) -> bool {
        self.dry
    }

    /// Write nothing: `post` and `post_full` only say they worked (`risk-check --dry-run`).
    pub fn dry(mut self, on: bool) -> Self {
        self.dry = on;
        self
    }

    /// Statuses only, as before `[checks]`.
    pub fn statuses(gh: &'a dyn Github) -> Self {
        Self::new(gh, None, CheckMode::Statuses)
    }

    /// The App client comes from `BOSUN_APP_TOKEN` (empty counts as unset), for `owner/repo`.
    pub fn from_env(gh: &'a dyn Github, mode: CheckMode, owner: &str, repo: &str) -> Self {
        let app = std::env::var(APP_TOKEN_ENV)
            .ok()
            .filter(|t| !t.trim().is_empty())
            .map(|t| Box::new(LiveGithub::with_token(t.trim(), owner, repo)) as Box<dyn Github>);
        Self::new(gh, app, mode)
    }

    /// `from_env` with the mode and repo of `ctx`.
    pub fn for_ctx(gh: &'a dyn Github, ctx: &crate::ctx::Ctx) -> Self {
        Self::from_env(gh, ctx.config.checks.mode, &ctx.repo.owner, &ctx.repo.name)
    }

    /// True when `dual` mode skipped a check run because the App token was missing.
    pub fn checks_skipped(&self) -> bool {
        self.skipped.get()
    }

    /// Write the verdict record comment on `n`. In `checks` mode the App writes it (only its
    /// records count then) and the body is defused: an App comment starts workflows, so it never
    /// carries `<!--`, `/bosun approve` or `/bosun recheck` (UC-B2-7).
    pub fn add_record(&self, n: u64, body: &str) -> Result<Comment> {
        if self.mode != CheckMode::Checks {
            return self.gh.add_comment(n, body);
        }
        let app = self.app.as_deref().ok_or_else(missing_token)?;
        app.add_comment(n, &defuse(body))
    }

    /// Post `state` (`pending`, `success`, `failure` or `error`) of `context` on `sha`.
    pub fn post(&self, sha: &str, context: &str, state: &str, description: &str) -> Result<()> {
        self.post_full(sha, context, state, description, description)
    }

    /// As `post`; the check run's summary is `summary` (the description plus the rules).
    pub fn post_full(
        &self,
        sha: &str,
        context: &str,
        state: &str,
        description: &str,
        summary: &str,
    ) -> Result<()> {
        if self.dry {
            return Ok(());
        }
        if self.mode != CheckMode::Checks {
            self.gh
                .create_status(sha, context, state, description, None)?;
        }
        if self.mode == CheckMode::Statuses {
            return Ok(());
        }
        let Some(app) = self.app.as_deref() else {
            if self.mode == CheckMode::Checks {
                return Err(missing_token());
            }
            if !self.skipped.replace(true) {
                eprintln!("bosun: {APP_TOKEN_ENV} is not set: check runs skipped (dual mode)");
            }
            return Ok(());
        };
        let posted = self.check_run(app, sha, context, state, description, summary);
        match posted {
            Err(e) if self.mode == CheckMode::Dual => {
                eprintln!(
                    "bosun: {context} check run not posted (dual mode): {}",
                    e.message
                );
                Ok(())
            }
            other => other,
        }
    }

    fn check_run(
        &self,
        app: &dyn Github,
        sha: &str,
        context: &str,
        state: &str,
        description: &str,
        summary: &str,
    ) -> Result<()> {
        let (run_state, title) = match state {
            "pending" => (RunState::Pending, description.to_string()),
            "success" => (RunState::Success, description.to_string()),
            "failure" => (RunState::Failure, description.to_string()),
            "error" => (RunState::Failure, format!("error: {description}")),
            other => {
                return Err(BosunError::error(
                    format!("gate state {other:?} is not one bosun posts"),
                    "use success, failure, pending or error",
                ));
            }
        };
        let run = GateRun {
            name: context,
            head_sha: sha,
            state: run_state,
            title: &title,
            summary,
        };
        let key = (sha.to_string(), context.to_string());
        let open = self.pending.borrow().get(&key).copied();
        // The verdict completes the run that `pending` opened in this process; a failed PATCH
        // keeps the id, so the `error` post retries that run instead of leaving it open.
        let id = match open {
            Some(id) => {
                app.update_check_run(id, &run)?;
                id
            }
            None => app.create_check_run(&run)?,
        };
        if run_state == RunState::Pending {
            self.pending.borrow_mut().insert(key, id);
        } else {
            self.pending.borrow_mut().remove(&key);
        }
        Ok(())
    }
}

/// `body` with no HTML comment opener and no `/bosun` command a workflow could match.
/// The record line (after the last blank line) is JSON-escaped instead (`\u002f`, `\u003c`), so it
/// reads back the same: a session name keeps matching its lease.
pub fn defuse(body: &str) -> String {
    let human = |t: &str| t.replace("<!--", "<! --").replace("/bosun", "/ bosun");
    match body.rsplit_once("\n\n") {
        Some((text, record)) if record.trim_start().starts_with('{') => {
            let record = record
                .replace('<', "\\u003c")
                .replace("/bosun", "\\u002fbosun");
            format!("{}\n\n{record}", human(text))
        }
        _ => human(body),
    }
}

fn missing_token() -> BosunError {
    BosunError::error(
        format!("[checks] mode = \"checks\" needs the gate App token in {APP_TOKEN_ENV}"),
        format!(
            "run the job in the `bosun-app` environment with the actions/create-github-app-token step (`client-id` from the `BOSUN_APP_CLIENT_ID` variable, `private-key` from `BOSUN_APP_KEY`) and pass its token as {APP_TOKEN_ENV}"
        ),
    )
}

/// The gate App bosun trusts for its own gates: set (from `[checks]` in `checks` mode) once per
/// process by `Ctx::load`; unset, the statuses are the authority, as in `statuses` and `dual`.
static TRUSTED_APP: OnceLock<Option<(u64, String)>> = OnceLock::new();

/// Record which App's check runs (and verdict records) count, from `[checks]`.
pub fn trust(checks: &Checks) {
    let app = match (checks.mode, checks.app_id, &checks.app_slug) {
        (CheckMode::Checks, Some(id), Some(slug)) => Some((id, slug.clone())),
        _ => None,
    };
    _ = TRUSTED_APP.set(app);
    let dual = match (checks.mode, checks.app_id) {
        (CheckMode::Dual, Some(id)) => Some(id),
        _ => None,
    };
    _ = DUAL_APP.set(dual);
}

/// In `dual` mode the gate App's id: its runs can only make a gate worse than its status.
static DUAL_APP: OnceLock<Option<u64>> = OnceLock::new();

fn trusted_app() -> Option<&'static (u64, String)> {
    TRUSTED_APP.get().and_then(Option::as_ref)
}

/// The gate App a reader checks a poster against: `[checks] app_id` in `dual` or `checks` mode;
/// `None` in `statuses` mode (nothing to check a status against).
pub fn gate_app_id() -> Option<u64> {
    trusted_app_id().or_else(|| DUAL_APP.get().copied().flatten())
}

/// In `checks` mode the id of the gate App whose check runs count; `None` in `statuses` mode.
pub fn trusted_app_id() -> Option<u64> {
    trusted_app().map(|(id, _)| *id)
}

/// Whether `[checks] mode = "checks"` is in force for this process.
pub fn checks_mode() -> bool {
    trusted_app().is_some()
}

/// In `checks` mode the login that writes trusted verdict records, `<app_slug>[bot]`.
pub fn record_bot() -> Option<String> {
    trusted_app().map(|(_, slug)| format!("{slug}[bot]"))
}

/// The author a verdict-record reader sees (UC-B2-4): in `checks` mode the App's comments read
/// as [`VERDICT_AUTHOR`] and `github-actions[bot]`'s as nobody; otherwise unchanged.
pub fn record_author(user: &str) -> String {
    match record_bot() {
        Some(bot) if user.eq_ignore_ascii_case(&bot) => VERDICT_AUTHOR.to_string(),
        Some(_) if user.eq_ignore_ascii_case(VERDICT_AUTHOR) => String::new(),
        _ => user.to_string(),
    }
}

/// Every post of the gates on `sha`, newest first, as statuses. In `checks` mode these are the
/// trusted App's check runs only (creator [`TRUSTED_CREATOR`]); statuses and other Apps' runs
/// never count (UC-B2-3). Otherwise the status history; in `dual` led by the newest per context
/// judged against the App's runs (as [`gate_statuses`]).
pub fn gate_history(gh: &dyn Github, sha: &str) -> Result<Vec<Status>> {
    let Some((id, _)) = trusted_app() else {
        let history = gh.status_history(sha)?;
        let Some(id) = DUAL_APP.get().copied().flatten() else {
            return Ok(history);
        };
        // dual: the newest per context, judged against the App (as `gate_statuses`), first.
        let mut seen = std::collections::HashSet::new();
        let newest = (history.iter())
            .filter(|s| seen.insert(s.context.clone()))
            .cloned()
            .collect();
        let mut out = worse_of_app(newest, dual_runs(gh, sha)?, id);
        out.extend(history);
        return Ok(out);
    };
    let mut runs: Vec<CheckRun> = (gh.check_runs_all(sha)?.into_iter())
        .filter(|r| r.app_id == *id)
        .collect();
    runs.sort_by_key(|r| std::cmp::Reverse(r.id));
    Ok(runs.into_iter().map(as_status).collect())
}

/// As [`gate_history`], the newest per context (the combined status in `statuses`). In `dual`
/// each context the gate App also posted takes the worse of its status and the App's newest run
/// (error > failure > pending > success): a status any workflow can post never outvotes the App.
pub fn gate_statuses(gh: &dyn Github, sha: &str) -> Result<Vec<Status>> {
    if trusted_app().is_none() {
        let statuses = gh.statuses(sha)?;
        return match DUAL_APP.get().copied().flatten() {
            Some(id) => Ok(worse_of_app(statuses, dual_runs(gh, sha)?, id)),
            None => Ok(statuses),
        };
    }
    let mut seen = std::collections::HashSet::new();
    let all = gate_history(gh, sha)?;
    Ok(all
        .into_iter()
        .filter(|s| seen.insert(s.context.clone()))
        .collect())
}

/// The check runs on `sha` for the `dual` check. A token without `checks: read` (a private repo's
/// workflow `GITHUB_TOKEN` that does not grant it) reads none: one stderr line, the statuses
/// alone, as before `dual` checked the poster.
/// Any other error (5xx, a rate limit, the network) fails the reader: falling back then would let
/// a forged status win again.
fn dual_runs(gh: &dyn Github, sha: &str) -> Result<Vec<CheckRun>> {
    match gh.check_runs_all(sha) {
        Err(e) if forbidden(&e.message) => {
            eprintln!(
                "bosun: dual mode: this token cannot read check runs ({}); statuses only",
                e.message
            );
            Ok(Vec::new())
        }
        r => r,
    }
}

/// A permission 403 (`GitHub GET <path>: 403 <message>`), not a rate limit.
fn forbidden(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains(": 403") && !m.contains("rate limit")
}

/// Each status whose context App `id` also posted becomes the worse of the two; a context only
/// the App posted is added. Other Apps' runs are ignored.
fn worse_of_app(mut statuses: Vec<Status>, runs: Vec<CheckRun>, id: u64) -> Vec<Status> {
    let rank = |s: &str| match s {
        "success" => 0,
        "pending" => 1,
        "failure" => 2,
        _ => 3,
    };
    let mut runs: Vec<CheckRun> = runs.into_iter().filter(|r| r.app_id == id).collect();
    runs.sort_by_key(|r| std::cmp::Reverse(r.id));
    let mut seen = std::collections::HashSet::new();
    for app in (runs.into_iter())
        .filter(|r| seen.insert(r.name.clone()))
        .map(as_status)
    {
        match statuses.iter_mut().find(|s| s.context == app.context) {
            Some(s) if rank(&app.state) > rank(&s.state) => *s = app,
            Some(_) => {}
            None => statuses.push(app),
        }
    }
    statuses
}

/// A check run read as a status: an `error:` title is `error`, an open run is `pending`.
pub(crate) fn as_status(r: CheckRun) -> Status {
    let state = match (r.status.as_str(), r.conclusion.as_deref()) {
        ("completed", Some("success")) => "success",
        ("completed", _) if r.title.starts_with("error:") => "error",
        ("completed", _) => "failure",
        _ => "pending",
    };
    Status {
        context: r.name,
        state: state.into(),
        description: r.title,
        creator: TRUSTED_CREATOR.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::records::Record;
    use crate::core::verdict::bot_verdict_json;

    #[test]
    fn a_defused_verdict_record_is_no_bot_verdict() {
        // bosun's own record comment (App token, a Bot) has "head_sha" but no ```json fence,
        // so verdict.yml never runs on it and `apply` finds no verdict in it.
        let line = Record::Verdict {
            session: "bot:claude[bot]".into(),
            head_sha: "sha42".into(),
            approved: true,
            confidence: Some(0.9),
            major: Some(0),
            model: None,
            relayed_by: None,
        }
        .to_line();
        let body = defuse(&format!("bosun verdict for sha42\n\n{line}"));
        assert!(body.contains("\"head_sha\""), "{body}");
        assert!(!body.contains("```json"), "{body}");
        assert_eq!(bot_verdict_json(&body), None);
    }

    #[test]
    fn only_a_permission_403_falls_back() {
        let e = |s: &str| format!("GitHub GET /repos/o/r/commits/a/check-runs: {s}");
        assert!(forbidden(&e("403 Resource not accessible by integration")));
        assert!(!forbidden(&e(
            "403 API rate limit exceeded for installation"
        )));
        assert!(!forbidden(&e("500 boom")));
        assert!(!forbidden(&e("429 slow down")));
    }
}
