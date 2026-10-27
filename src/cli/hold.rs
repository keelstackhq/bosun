//! `bosun hold <task issue> "<what the owner must do>" [--phone]`: an agent met a human step
//! nobody listed. It does not wait: the step becomes a box in the plan's owner checklist, a
//! record on the task keeps the hold, and the task goes back to `ready` for the claim gate to
//! keep (design F6).

use std::path::Path;

use serde_json::json;

use super::prereqs::{Env, check_plan_path, post};
use super::prereqs_gate::plan_of_task;
use super::release::{release_to, require_lease};
use super::{HoldArgs, LessonAddArgs, LessonCmd};
use crate::app::{App, Result, default_branch, kill_switch, notes, task_meta};
use crate::core::plan;
use crate::core::prereqs::{self as pq, HoldBox};
use crate::core::records::{Record, parse_record};
use crate::core::state::State;
use crate::core::text::{has_hidden, inert};
use crate::output::{BosunError, CmdResult, Outcome};

const MAX_TEXT: usize = 300;

pub fn run(dir: Option<&Path>, args: &HoldArgs) -> CmdResult {
    let app = App::load(dir)?;
    let (gh, n, session) = (app.gh.as_ref(), args.issue, &app.ctx.session);
    kill_switch(gh)?;
    let step = args.text.trim();
    check_text(step)?;
    let issue = gh.get_issue(n)?.ok_or_else(|| {
        BosunError::error(
            format!("issue #{n} does not exist"),
            "check the number; `bosun status` lists the tasks",
        )
    })?;
    let only_plan = || {
        BosunError::error(
            format!("#{n} is not a plan task, so only plan tasks can be held"),
            "hold the plan task you are working on; for hand-made work use `bosun ask`",
        )
    };
    let meta = task_meta(&issue)
        .filter(|m| m.epic.is_some())
        .ok_or_else(only_plan)?;
    let plan_path = plan_of_task(gh, &meta)?.ok_or_else(only_plan)?;
    check_plan_path(&plan_path)?;
    let comments = notes(gh, n)?;
    require_lease(n, session, &comments)?;

    let mut warnings = Vec::new();
    let env = Env::load(&app, dir, &mut warnings)?;
    env.one_issue(&plan_path)?;
    let existing = env.issue_of(&plan_path);
    let (rest, mut boxes) = existing.map_or_else(
        || (String::new(), vec![]),
        |i| pq::split_found(i.body_text()),
    );

    // X<n>: one more than the highest number on the task's hold records and in the checklist.
    let on_task = comments.iter().filter_map(|c| match parse_record(&c.body) {
        Some(Record::Hold { id, .. }) => pq::hold_number(&id),
        _ => None,
    });
    let in_issue = existing
        .into_iter()
        .flat_map(|i| pq::read_issue(i.body_text()));
    let top = on_task
        .chain(in_issue.filter_map(|t| pq::hold_number(&t.id)))
        .max();
    let id = format!("X{}", top.unwrap_or(0) + 1);
    if !pq::is_hold_id(&id) {
        return Err(BosunError::error(
            format!("{id} is not a valid hold id"),
            "ask the owner to clear the found-during-the-work boxes of the checklist",
        ));
    }

    // The durable copy first: the gate reads it even if the checklist write fails.
    let on = if args.phone { "phone" } else { "computer" };
    let record = Record::Hold {
        session: session.clone(),
        id: id.clone(),
        step: step.to_string(),
        on: on.into(),
    };
    let body = format!(
        "{session} found an owner step nobody listed ({id}, {on}): {}\n\n{}",
        inert(step, MAX_TEXT),
        record.to_line()
    );
    gh.add_comment(n, &body)?;

    boxes.push(HoldBox::new(&id, step, n, args.phone));
    let (rest, title) = match existing {
        Some(_) => (rest, String::new()),
        None => {
            let title = plan_title(&app, &plan_path)?;
            (pq::render_issue(&title, &plan_path, &[], &[]), title)
        }
    };
    let body = pq::with_found(&rest, &boxes);
    let (number, _) = post(gh, existing, &title, &body, 1, true)?.expect("an issue with a box");
    let url = format!("https://github.com/{}/issues/{number}", app.ctx.repo);

    release_to(gh, n, &issue, session, &comments, State::Ready)?;
    let lesson_error = lesson(dir, &meta.item, &meta.task, step);
    Ok(Outcome::data(json!({"issue": n, "id": id,
        "checklist": {"number": number, "url": url},
        "released": true, "lesson_error": lesson_error, "warnings": warnings})))
}

/// Non-empty, at most 300 characters, nothing that hides or fakes markup.
fn check_text(t: &str) -> Result<()> {
    let bad = if t.is_empty() {
        "is empty"
    } else if t.chars().count() > MAX_TEXT {
        "is longer than 300 characters"
    } else if t.contains("<!--") || t.contains("-->") {
        "has a comment marker"
    } else if has_hidden(t) {
        "has control or hidden characters"
    } else {
        return Ok(());
    };
    Err(BosunError::usage(
        format!("the hold text {bad}"),
        "say in one plain sentence (at most 300 characters, one line) what the owner must do",
    ))
}

/// The plan's title, from the plan file on the default branch.
fn plan_title(app: &App, plan_path: &str) -> Result<String> {
    let gh = app.gh.as_ref();
    let branch = default_branch(gh)?;
    let fail = |why: String| {
        BosunError::error(
            format!("{plan_path}: {why}"),
            "merge the plan to the default branch, then hold again",
        )
    };
    let text = gh
        .file_at(plan_path, &branch)?
        .ok_or_else(|| fail(format!("not on {branch}")))?;
    let parsed = plan::parse(&text).map_err(|e| fail(e[0].message.clone()))?;
    Ok(parsed.title)
}

/// A lesson that the plan missed a step; best effort, the error text on failure.
fn lesson(dir: Option<&Path>, item: &str, task: &str, step: &str) -> Option<String> {
    let text = format!(
        "Task {item}/{task} needed an owner step the plan did not list; declare such steps in `## Prerequisites`: {}",
        inert(step, 160)
    );
    let args = LessonAddArgs {
        text,
        scope: vec![],
        why: Some("an agent met the step mid-run and had to hold the task".into()),
        evidence: vec![],
        upstream: false,
    };
    super::lesson::run(dir, &LessonCmd::Add(args))
        .err()
        .map(|e| e.message)
}
