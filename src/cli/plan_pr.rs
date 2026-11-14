//! `bosun plan pr <plan> "<why>" --source <s>`: the PR of a plan amendment in one step. It
//! opens (or finds) the intake, claims it (its touch is the plan file) and runs `bosun pr` for
//! it. Each step is the command itself with its own checks; a re-run finds what is done.

use std::path::Path;

use serde_json::{Value, json};

use super::{ClaimArgs, IntakeArgs, PlanPrArgs, PrArgs};
use crate::app::{App, kill_switch};
use crate::core::state::{State, state_of};
use crate::output::{BosunError, CmdResult, Outcome};

pub fn run(dir: Option<&Path>, a: &PlanPrArgs) -> CmdResult {
    // The plan file is the PR's whole touch: one `.md` file under `plans/`, checked before
    // anything is read or written.
    super::intake::check_plan_path(&a.file)?;
    if !a.file.starts_with("plans/") {
        return Err(BosunError::usage(
            format!("plan path {:?} is not under plans/", a.file),
            "pass a plan file under plans/, for example plans/bosun.md",
        ));
    }
    let app = App::load(dir)?;
    kill_switch(app.gh.as_ref())?;
    let intake = super::intake::open(
        &app,
        &IntakeArgs {
            text: a.why.clone(),
            source: a.source.clone(),
            item: a.item.clone(),
            title: a.title.clone(),
            plan: Some(a.file.clone()),
            cmd: None,
        },
    )?;
    let n = intake.data["issue"].as_u64().ok_or_else(|| {
        BosunError::error(
            "intake returned no issue number",
            "run `bosun intake` and `bosun claim` by hand",
        )
    })?;
    let mut data = json!({"intake": intake.data, "claim": null, "pr": null});

    // Claim unless the intake is past it already (a re-run after `bosun pr` moved it to review).
    let issue = app.gh.get_issue(n)?;
    let past_claim = issue.as_ref().is_some_and(|i| {
        matches!(
            state_of(&i.labels),
            Some(State::Review | State::Fixing | State::Queued)
        )
    });
    if !past_claim {
        let claim = super::claim::run(
            dir,
            &ClaimArgs {
                issue: n,
                touch: vec![],
            },
        )
        .map_err(|e| with_steps(e, &data))?;
        data["claim"] = claim.data;
    }

    let pr = super::pr::run(
        dir,
        &PrArgs {
            issue: Some(n),
            pr: None,
            title: None,
            ready: a.auto_merge,
            bundle: vec![],
        },
    )
    .map_err(|e| with_steps(e, &data))?;
    data["pr"] = pr.data;
    Ok(Outcome {
        data,
        next: pr.next,
    })
}

/// A failed step keeps the data of the steps before it (`data.failed` is the step's own data).
fn with_steps(mut e: BosunError, done: &Value) -> BosunError {
    let mut d = done.clone();
    if let Some(own) = e.data.take() {
        d["failed"] = own;
    }
    e.data = Some(d);
    e
}
