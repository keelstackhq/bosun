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
        }
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
pub fn defuse(body: &str) -> String {
    body.replace("<!--", "<! --").replace("/bosun", "/ bosun")
}

fn missing_token() -> BosunError {
    BosunError::error(
        format!("[checks] mode = \"checks\" needs the gate App token in {APP_TOKEN_ENV}"),
        format!(
            "run the job in the `bosun-app` environment with the actions/create-github-app-token step and pass its token as {APP_TOKEN_ENV}"
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
}

fn trusted_app() -> Option<&'static (u64, String)> {
    TRUSTED_APP.get().and_then(Option::as_ref)
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
/// never count (UC-B2-3). Otherwise the status history, as before.
pub fn gate_history(gh: &dyn Github, sha: &str) -> Result<Vec<Status>> {
    let Some((id, _)) = trusted_app() else {
        return gh.status_history(sha);
    };
    let mut runs: Vec<CheckRun> = (gh.check_runs_all(sha)?.into_iter())
        .filter(|r| r.app_id == *id)
        .collect();
    runs.sort_by_key(|r| std::cmp::Reverse(r.id));
    Ok(runs.into_iter().map(as_status).collect())
}

/// As [`gate_history`], the newest per context (the combined status in `statuses` and `dual`).
pub fn gate_statuses(gh: &dyn Github, sha: &str) -> Result<Vec<Status>> {
    if trusted_app().is_none() {
        return gh.statuses(sha);
    }
    let mut seen = std::collections::HashSet::new();
    let all = gate_history(gh, sha)?;
    Ok(all
        .into_iter()
        .filter(|s| seen.insert(s.context.clone()))
        .collect())
}

fn as_status(r: CheckRun) -> Status {
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
