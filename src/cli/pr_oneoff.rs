//! `bosun pr --one-off --title T --touch GLOB... [--ready]`: a PR for work nobody planned (a
//! request typed in Slack, a quick fix). It opens a ready task of kind `oneoff` whose touch is
//! the given globs, claims it as this session and runs `bosun pr` for it: each step is the
//! command itself with its own checks, so a gated touch, an overlap or a red main refuse as
//! they do for any task. Nothing is created before the arguments are checked.

use std::path::Path;

use serde_json::json;

use super::{ClaimArgs, PrArgs};
use crate::app::{App, base_files, default_branch, gated_from_default, kill_switch};
use crate::core::glob::Glob;
use crate::core::meta::TaskMeta;
use crate::core::risk::autonomous_allow_globs;
use crate::core::shape::{ONE_OFF_ITEM, touch_gated_in};
use crate::core::state::State;
use crate::output::{BosunError, CmdResult, Outcome};

pub fn run(dir: Option<&Path>, a: &PrArgs) -> CmdResult {
    let title = a.title.as_deref().map(str::trim).unwrap_or_default();
    if title.is_empty() || a.touch.is_empty() {
        return Err(BosunError::usage(
            "--one-off needs --title and at least one --touch",
            "pass --title \"<what the PR does>\" and --touch GLOB (repeatable) for the paths it edits",
        ));
    }
    super::refuse_secrets("--touch", &a.touch)?;
    // The glob is written into the task marker (`<!-- bosun:task {json} -->`): it must not close
    // or reopen a comment there.
    if let Some(g) = a
        .touch
        .iter()
        .find(|g| g.contains("<!--") || g.contains("-->"))
    {
        return Err(BosunError::usage(
            format!("--touch `{g}` contains an HTML comment marker"),
            "pass plain path globs; `<!--` and `-->` are not allowed in a touch",
        ));
    }
    let app = App::load(dir)?;
    kill_switch(app.gh.as_ref())?;
    check_touch(&app, &a.touch)?;
    let meta = TaskMeta {
        item: ONE_OFF_ITEM.into(),
        task: title.into(),
        epic: None,
        touch: a.touch.clone(),
        depends: vec![],
        kind: "feature".into(),
        risk: "normal".into(),
        allow: vec![],
        capabilities: vec![],
        gated: false,
        size: None,
    };
    let labels = [
        crate::core::shape::ONE_OFF_LABEL.to_string(),
        State::Ready.label(),
    ];
    for l in &labels {
        app.gh
            .ensure_label(l, "ededed", "opened by bosun pr --one-off")?;
    }
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    // The marker is machine-built from the globs; the title is plain text of the issue.
    let body = format!(
        "One-off task opened by `bosun pr --one-off` ({}).\n\n<!-- bosun:task {} -->",
        app.ctx.session,
        serde_json::to_string(&meta).expect("meta serializes")
    );
    let issue = app.gh.create_issue(title, &body, &refs)?;
    let n = issue.number;
    eprintln!("bosun: one-off task #{n} opened");
    let claim = super::claim::run(
        dir,
        &ClaimArgs {
            issue: n,
            touch: vec![],
        },
    )?;
    let pr = super::pr::run(
        dir,
        &PrArgs {
            issue: Some(n),
            pr: None,
            title: a.title.clone(),
            ready: a.ready,
            bundle: vec![],
            one_off: false,
            touch: vec![],
        },
    )?;
    // The data is the PR's own (the `pr` schema), plus the task and its claim.
    let mut data = pr.data;
    data["task"] = json!(n);
    data["claim"] = claim.data;
    Ok(Outcome {
        data,
        next: pr.next,
    })
}

/// Every touch glob lies inside the autonomous allow set of the default branch (`[approvals]
/// autonomous_allow`, else the built-in set) and overlaps no gated path of its risk rules: work
/// nobody planned never reaches what the plan or the owner decides. Nothing is created before.
fn check_touch(app: &App, touch: &[String]) -> Result<(), BosunError> {
    let gh = app.gh.as_ref();
    let base = default_branch(gh).unwrap_or_else(|_| "main".to_string());
    let allow = super::risk_check::autonomous_allow(gh, &base)?;
    let allow = autonomous_allow_globs(allow.as_deref());
    let gated = gated_from_default(gh)?;
    for raw in touch {
        let g = Glob::parse(raw).map_err(|e| BosunError::usage(e, "pass plain path globs"))?;
        if !allow.iter().any(|a| a.covers(&g)) {
            return Err(BosunError::conflict(
                format!("--touch `{raw}` is outside the paths a one-off may edit on {base}"),
                "a one-off covers only the autonomous allow set (src/**, tests/**, docs/**, changelog.d/**, .changeset/*.md, or the owner's `[approvals] autonomous_allow`): plan the work instead (`bosun plan import`)",
            ));
        }
    }
    // As a plan task: a gated glob over the touch, wildcards resolved against the base's files.
    let files = base_files(gh)?;
    let hit = touch_gated_in(touch, &gated, files.as_deref())
        .map_err(|e| BosunError::conflict(e, "name the files the PR edits, without wildcards"))?;
    if hit {
        return Err(BosunError::conflict(
            "--touch reaches a gated path (a risk rule of the default branch)",
            "gated work needs a planned task with the owner's approval: plan it (`bosun plan import`), or narrow --touch to files outside the gated paths",
        ));
    }
    Ok(())
}
