//! The one writer of the gate contexts `risk-check`, `review-verdict` and `main-open` (spec §15):
//! a commit status, a check run of the gate App, or both, by `[checks] mode`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use super::{GateRun, Github, LiveGithub, Result, RunState};
use crate::config::CheckMode;
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
        let open = self.pending.borrow_mut().remove(&key);
        // The verdict completes the run that `pending` opened in this process.
        let id = match open {
            Some(id) => {
                app.update_check_run(id, &run)?;
                id
            }
            None => app.create_check_run(&run)?,
        };
        if run_state == RunState::Pending {
            self.pending.borrow_mut().insert(key, id);
        }
        Ok(())
    }
}

fn missing_token() -> BosunError {
    BosunError::error(
        format!("[checks] mode = \"checks\" needs the gate App token in {APP_TOKEN_ENV}"),
        format!(
            "run the job in the `bosun-app` environment with the actions/create-github-app-token step and pass its token as {APP_TOKEN_ENV}"
        ),
    )
}
