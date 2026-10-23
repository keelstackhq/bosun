//! `bosun lesson triage`: the curator's decision on a lesson issue (spec §6). Blocking sets
//! process-red, mechanizable opens a process-fix task, advice goes into `docs/lessons.toml`.

use std::path::Path;

use serde_json::json;

use super::LessonTriageArgs;
use super::lesson::{FILE, LESSON_LABEL, cap_error, load, save, today};
use super::status::PROCESS_RED_LABEL;
use crate::app::{App, kill_switch};
use crate::core::lessons;
use crate::core::meta::TaskMeta;
use crate::output::{BosunError, CmdResult, Outcome};

const KINDS: [&str; 3] = ["blocking", "mechanizable", "advice"];

fn label(kind: &str) -> String {
    format!("{LESSON_LABEL}/{kind}")
}

pub fn run(dir: Option<&Path>, a: &LessonTriageArgs) -> CmdResult {
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    kill_switch(gh)?;
    let n = a.issue;
    let lesson = gh
        .get_issue(n)?
        .filter(|i| !i.pull_request && i.state == "open" && i.labels.iter().any(|l| l == LESSON_LABEL))
        .ok_or_else(|| {
            BosunError::error(
                format!("#{n} is not an open `{LESSON_LABEL}` issue"),
                format!("pass an open lesson issue; `bosun status` and the label {LESSON_LABEL} list them"),
            )
        })?;
    let title = lesson
        .title
        .strip_prefix("lesson: ")
        .unwrap_or(&lesson.title);
    let kind = a.triage.as_str();
    let (colour, text) = ("5319e7", "triage of a lesson issue");
    let mut data = json!({"issue": n, "as": kind});
    let mut store = None;
    let comment;
    match kind {
        "blocking" => {
            gh.ensure_label(
                PROCESS_RED_LABEL,
                "b60205",
                "a blocking lesson pauses waves",
            )?;
            gh.ensure_label(&label(kind), colour, text)?;
            gh.add_labels(n, &[&label(kind), PROCESS_RED_LABEL])?;
            comment = "Triaged as blocking: process is red until its process-fix merges (spec §6)."
                .into();
            data["process_red"] = json!(n);
        }
        "mechanizable" => {
            if a.touch.is_empty() {
                return Err(BosunError::usage(
                    "mechanizable needs the paths the process-fix changes",
                    "add --touch GLOB (repeatable) to `bosun lesson triage`",
                ));
            }
            let mut labels = vec![
                "bosun:kind/process-fix".to_string(),
                "bosun:source/lesson".into(),
                "bosun:state/ready".into(),
            ];
            labels.extend(a.area.iter().map(|x| format!("bosun:area/{x}")));
            for l in &labels {
                gh.ensure_label(l, "ededed", "opened by lesson triage")?;
            }
            let meta = TaskMeta {
                item: "lesson".into(),
                task: format!("#{n}"),
                epic: None,
                touch: a.touch.clone(),
                depends: vec![],
                kind: "process-fix".into(),
                risk: "normal".into(),
                allow: vec![],
                capabilities: vec![],
            };
            let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
            let json = serde_json::to_string(&meta).expect("meta serializes");
            let intro =
                format!("Mechanize lesson #{n}: turn it into a gate, a hook or a check (spec §6).");
            let task = gh.create_issue(
                &format!("process-fix: {title}"),
                &format!("{intro}\n\n<!-- bosun:task {json} -->"),
                &refs,
            )?;
            // The task number is known only now, so the body is completed in a second call.
            let body = format!(
                "{intro} The PR closes both: `Closes #{}` and `Closes #{n}`.\n\n<!-- bosun:task {json} -->",
                task.number
            );
            gh.update_issue(
                task.number,
                crate::github::IssuePatch {
                    body: Some(body),
                    ..Default::default()
                },
            )?;
            gh.ensure_label(&label(kind), colour, text)?;
            gh.add_labels(n, &[&label(kind)])?;
            comment = format!(
                "Triaged as mechanizable: process-fix task #{}.",
                task.number
            );
            data["task"] = json!(task.number);
        }
        _ => {
            let Some(rule) = a.rule.as_deref() else {
                return Err(BosunError::usage(
                    "advice needs the rule text",
                    "add --rule TEXT (and --scope GLOB) to `bosun lesson triage`",
                ));
            };
            let mut s = load(&app.ctx.dir)?;
            let id = lessons::add(
                &mut s,
                rule,
                a.scope.clone(),
                title,
                vec![format!("#{n}")],
                &today(&app.ctx),
            )
            .map_err(cap_error)?;
            comment = format!(
                "Triaged as advice: {id} in {FILE}; the store PR closes this issue (`Closes #{n}`)."
            );
            gh.ensure_label(&label(kind), colour, text)?;
            gh.add_labels(n, &[&label(kind)])?;
            data["lesson"] = json!(id);
            store = Some(s);
        }
    }
    // A re-triage drops the other triage labels, and process-red when leaving blocking.
    for other in KINDS.iter().filter(|k| **k != kind) {
        gh.remove_label(n, &label(other))?;
    }
    if kind != "blocking" {
        gh.remove_label(n, PROCESS_RED_LABEL)?;
    }
    gh.add_comment(n, &comment)?;
    if let Some(s) = store {
        save(&app.ctx.dir, &s)?;
    }
    Ok(Outcome::data(data))
}
