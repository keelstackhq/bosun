//! `bosun intake`: new work enters only as a plan amendment (spec §4). The command opens one
//! kind/plan issue that asks for the amendment; only the merged plan task makes tasks ready.

use std::path::Path;

use serde_json::json;

use super::plan::plan_errors;
use super::{IntakeArgs, IntakeCmd};
use crate::app::{App, kill_switch};
use crate::core::meta::TaskMeta;
use crate::core::plan;
use crate::core::state::State;
use crate::github::IssueState;
use crate::output::{BosunError, CmdResult, Outcome};

/// Text made safe to fence as data: it cannot close the fence, open an HTML comment marker or
/// forge a record line, and it is capped at 4000 characters.
pub(crate) fn fence_text(text: &str) -> String {
    text.chars()
        .take(4000)
        .collect::<String>()
        .replace('`', "\u{02cb}")
        .replace("<!--", "<!-\u{200b}-")
        .replace(r#"{"bosun""#, r#"{ "bosun""#)
}

pub fn run(dir: Option<&Path>, a: &IntakeArgs) -> CmdResult {
    match &a.cmd {
        Some(IntakeCmd::Check(c)) => return super::intake_file::check(dir, c),
        Some(IntakeCmd::Sign(c)) => return super::intake_file::sign(dir, c),
        Some(IntakeCmd::Verify(c)) => return super::intake_file::verify(dir, c),
        None => {}
    }
    // Usage errors come before any repo or GitHub access, as they always did.
    args_ok(a)?;
    let app = App::load(dir)?;
    kill_switch(app.gh.as_ref())?;
    open(&app, a)
}

/// Required in the old form; clap cannot say so next to the subcommands.
fn args_ok(a: &IntakeArgs) -> Result<(), BosunError> {
    let (what, fix) = if a.text.trim().is_empty() {
        (
            "the intake text is empty",
            "pass what is wanted as the first argument: bosun intake \"<text>\" --source human",
        )
    } else if a.source.is_empty() {
        (
            "--source is required",
            "pass where the request came from: --source slack|lesson|flake|human",
        )
    } else {
        return Ok(());
    };
    Err(BosunError::usage(what, fix))
}

/// Open (or find) the kind/plan issue of a request; the kill switch is the caller's.
pub(crate) fn open(app: &App, a: &IntakeArgs) -> CmdResult {
    args_ok(a)?;
    let text = a.text.trim();
    let file = a
        .plan
        .clone()
        .unwrap_or_else(|| format!("plans/{}.md", app.ctx.config.project));
    check_plan_path(&file)?;
    let area = plan_area(&app.ctx.dir, &file, a.item.as_deref())?;
    let first = text.lines().next().unwrap_or(text).trim();
    let title_text = a
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or(first);
    let title = format!(
        "intake: {}",
        title_text
            .chars()
            .take(80)
            .collect::<String>()
            .replace('`', "\u{02cb}")
            .replace('@', "@\u{200b}")
            .replace("<!--", "<!-\u{200b}-")
    );
    let kind = "bosun:kind/plan";
    let source = format!("bosun:source/{}", a.source);

    let open = app.gh.list_issues(&[kind, &source], IssueState::Open)?;
    if let Some(i) = open.iter().find(|i| i.title == title) {
        return Ok(Outcome::data(data(i.number, false, a, &file)));
    }

    let meta = TaskMeta {
        item: a.item.clone().unwrap_or_else(|| "intake".into()),
        task: "intake".into(),
        epic: None,
        touch: vec![file.clone()],
        depends: vec![],
        kind: "plan".into(),
        risk: "normal".into(),
        allow: vec!["skip".into()],
        capabilities: vec![],
    };
    // The metadata marker comes first and the request text cannot carry a marker or close its
    // fence; readers also take the last marker (core::meta).
    let quoted = text.replace("```", "'''").replace("<!--", "<!-\u{200b}-");
    let body = format!(
        "<!-- bosun:task {} -->\n\n```text\n{quoted}\n```\n\nAmend {file}: add or change an item/task for this request (docs/plan-format.md), \
         open a kind/plan PR that closes this issue, and run `bosun plan amend {file}` after it \
         merges; only then do the new tasks become ready (spec §4).\n",
        serde_json::to_string(&meta).expect("meta serializes")
    );
    let ready = State::Ready.label();
    let mut labels = vec![kind.to_string(), source, ready];
    labels.extend(area.map(|x| format!("bosun:area/{x}")));
    for l in &labels {
        app.gh.ensure_label(l, "5319e7", "bosun intake label")?;
    }
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    let issue = app.gh.create_issue(&title, &body, &refs)?;
    Ok(Outcome::data(data(issue.number, true, a, &file)))
}

/// The plan path becomes the intake's `touch` verbatim, so it must name one file: a
/// repo-relative `.md` path with no glob characters, no `.` or `..` part and no backslash.
pub(crate) fn check_plan_path(file: &str) -> Result<(), BosunError> {
    let why = if file.chars().any(|c| "*?[]{}!".contains(c)) {
        Some("has a glob character")
    } else if file.starts_with('/') || file.contains('\\') || file.contains(':') {
        Some("is not a repo-relative path")
    } else if file
        .split('/')
        .any(|p| p.is_empty() || p == "." || p == "..")
    {
        Some("has an empty, `.` or `..` part")
    } else if !file.ends_with(".md") {
        Some("is not a .md file")
    } else {
        None
    };
    match why {
        None => Ok(()),
        Some(why) => Err(BosunError::usage(
            format!("plan path {file:?} {why}"),
            "pass the repo-relative path of one plan file, for example plans/bosun.md",
        )),
    }
}

fn data(issue: u64, created: bool, a: &IntakeArgs, file: &str) -> serde_json::Value {
    json!({"issue": issue, "created": created, "source": a.source,
        "item": a.item, "plan": file})
}

/// Check the plan file and return the area of `item`; a missing file or item is a usage error.
fn plan_area(root: &Path, file: &str, item: Option<&str>) -> Result<Option<String>, BosunError> {
    let text = std::fs::read_to_string(root.join(file)).map_err(|e| {
        BosunError::usage(
            format!("cannot read the plan {file}: {e}"),
            "pass --plan with the path of the plan file, for example plans/bosun.md",
        )
    })?;
    let p = plan::parse(&text).map_err(|errs| plan_errors(file, &errs))?;
    let Some(id) = item else { return Ok(None) };
    match p.items.iter().find(|i| i.id == id) {
        Some(i) => Ok(Some(i.area.clone())),
        None => Err(BosunError::usage(
            format!("{file} has no item {id}"),
            format!(
                "pass --item with one of: {}",
                p.items
                    .iter()
                    .map(|i| i.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        )),
    }
}
