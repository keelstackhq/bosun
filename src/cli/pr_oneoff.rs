//! `bosun pr --one-off --title T --touch GLOB... [--ready]`: a PR for work nobody planned (a
//! request typed in Slack, a quick fix). It opens a ready task of kind `oneoff` whose touch is
//! the given globs, claims it as this session and runs `bosun pr` for it: each step is the
//! command itself with its own checks, so a gated touch, an overlap or a red main refuse as
//! they do for any task. Nothing is created before the arguments are checked.

use std::path::Path;

use serde_json::json;

use super::{ClaimArgs, PrArgs};
use crate::app::{App, kill_switch};
use crate::core::meta::TaskMeta;
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
    let app = App::load(dir)?;
    kill_switch(app.gh.as_ref())?;
    let meta = TaskMeta {
        item: "oneoff".into(),
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
    let labels = ["bosun:source/oneoff".to_string(), State::Ready.label()];
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
