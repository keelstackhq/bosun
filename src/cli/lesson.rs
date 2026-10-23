//! `bosun lesson add|hit`: the lessons store `docs/lessons.toml` (spec §6). `add` and `hit`
//! work on the local file; `hit --issue` also posts a record, which needs GitHub.

use std::path::Path;

use serde_json::json;

use super::{LessonAddArgs, LessonCaptureArgs, LessonCmd, LessonHitArgs};
use crate::app::{App, kill_switch, notes};
use crate::core::lessons::{self, Store};
use crate::core::records::{Record, parse_record};
use crate::core::signature::{lesson_marker, normalise, parse_lesson_marker, signature};
use crate::ctx::Ctx;
use crate::github::{Github, IssueState};
use crate::output::{BosunError, CmdResult, Outcome};

pub const FILE: &str = "docs/lessons.toml";
const HEADER: &str = "# bosun lessons store, spec §6\n\n";

pub fn run(dir: Option<&Path>, cmd: &LessonCmd) -> CmdResult {
    match cmd {
        LessonCmd::Add(a) => add(dir, a),
        LessonCmd::Hit(a) => hit(dir, a),
        LessonCmd::Capture(a) => capture(dir, a),
        LessonCmd::Triage(a) => super::lesson_triage::run(dir, a),
    }
}

/// The store of a repo root; a missing file is an empty store.
pub fn load(root: &Path) -> Result<Store, BosunError> {
    match std::fs::read_to_string(root.join(FILE)) {
        Ok(t) => toml::from_str(&t).map_err(|e| {
            BosunError::error(
                format!("{FILE}: {}", e.message()),
                format!("fix the TOML in {FILE}; entries are [[lesson]] tables (spec §6)"),
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(e) => Err(BosunError::error(
            format!("cannot read {FILE}: {e}"),
            format!("check that {FILE} is a readable file"),
        )),
    }
}

/// Write the store atomically (temp file, then rename).
pub(super) fn save(root: &Path, store: &Store) -> Result<(), BosunError> {
    let fail = |e: std::io::Error| {
        BosunError::error(
            format!("cannot write {FILE}: {e}"),
            "check the permissions of the docs directory",
        )
    };
    let text = format!(
        "{HEADER}{}",
        toml::to_string(store).expect("lessons serialize")
    );
    let path = root.join(FILE);
    std::fs::create_dir_all(path.parent().expect("has parent")).map_err(fail)?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(fail)?;
    std::fs::rename(&tmp, &path).map_err(fail)
}

pub(super) fn today(ctx: &Ctx) -> String {
    ctx.now.to_string()[..10].to_string()
}

fn add(dir: Option<&Path>, a: &LessonAddArgs) -> CmdResult {
    let ctx = Ctx::load(dir)?;
    let mut store = load(&ctx.dir)?;
    let id = lessons::add(
        &mut store,
        &a.text,
        a.scope.clone(),
        a.why.as_deref().unwrap_or(""),
        a.evidence.clone(),
        &today(&ctx),
    )
    .map_err(cap_error)?;
    save(&ctx.dir, &store)?;
    Ok(Outcome::data(
        json!({"id": id, "count": store.lesson.len(), "global_count": store.global_count()}),
    ))
}

/// The exit-6 error of a store at a cap.
pub(super) fn cap_error(cap: lessons::CapError) -> BosunError {
    let (n, what) = if cap.global {
        (lessons::MAX_GLOBAL, "global lessons")
    } else {
        (lessons::MAX, "lessons")
    };
    BosunError::limit(
        format!("{FILE} already has {n} {what}"),
        "mechanize, merge or retire a lesson (spec §6)",
    )
}

fn hit(dir: Option<&Path>, a: &LessonHitArgs) -> CmdResult {
    let ctx = Ctx::load(dir)?;
    let mut store = load(&ctx.dir)?;
    if !lessons::hit(&mut store, &a.id, &today(&ctx)) {
        return Err(BosunError::error(
            format!("no lesson {} in {FILE}", a.id),
            format!("check the id in {FILE}"),
        ));
    }
    let hits = store
        .lesson
        .iter()
        .find(|l| l.id == a.id)
        .map_or(0, |l| l.hits);
    let mut comment = json!(null);
    if let Some(issue) = a.issue {
        let app = App::load(dir)?;
        kill_switch(app.gh.as_ref())?;
        let record = Record::LessonHit {
            session: ctx.session.clone(),
            id: a.id.clone(),
        };
        comment = json!(app.gh.add_comment(issue, &record.to_line())?.id);
    }
    save(&ctx.dir, &store)?;
    Ok(Outcome::data(
        json!({"id": a.id, "hits": hits, "comment_id": comment}),
    ))
}

/// `lesson capture`: the failure text from stdin or a file becomes a lesson issue, or a hit on
/// the open lesson with the same signature (spec §6).
fn capture(dir: Option<&Path>, a: &LessonCaptureArgs) -> CmdResult {
    let read = if a.from == "-" {
        let mut t = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut t).map(|_| t)
    } else {
        std::fs::read_to_string(&a.from)
    };
    let text = read.map_err(|e| {
        BosunError::usage(
            format!("cannot read the failure text from {}: {e}", a.from),
            "pass the failure text on stdin or with --from FILE",
        )
    })?;
    let Some(first) = text.lines().map(str::trim).find(|l| !l.is_empty()) else {
        return Err(BosunError::usage(
            "the failure text is empty",
            "pipe the failure text to `bosun lesson capture --kind <kind> --from -`",
        ));
    };
    let title = a
        .title
        .clone()
        .unwrap_or_else(|| first.chars().take(80).collect());
    let app = App::load(dir)?;
    kill_switch(app.gh.as_ref())?;
    let c = capture_lesson(
        app.gh.as_ref(),
        &app.ctx.session,
        &a.kind,
        &title,
        &text,
        a.issue,
    )?;
    Ok(Outcome::data(
        json!({"issue": c.issue, "signature": c.signature,
        "created": c.created, "hits": c.hits, "kind": a.kind}),
    ))
}

/// Label of lesson issues (spec §6).
pub const LESSON_LABEL: &str = "bosun:lesson";

/// What [`capture_lesson`] did.
#[derive(Debug, Clone, PartialEq)]
pub struct Captured {
    /// The lesson issue, new or existing.
    pub issue: u64,
    /// The failure signature (16 hex characters).
    pub signature: String,
    /// True when this call opened the issue.
    pub created: bool,
    /// `lesson-seen` records on the issue after this call; 0 for a new one.
    pub hits: usize,
}

/// Capture a failure as a lesson (spec §6). The signature is [`signature`] of `kind` and `text`.
/// An open `bosun:lesson` issue with that signature gets a `Seen again` comment holding a
/// `lesson-seen` record; otherwise a new lesson issue opens. `task` is the issue where the
/// failure was seen. Other commands (stop rules, budget) call this; callers check the kill
/// switch first.
pub fn capture_lesson(
    gh: &dyn Github,
    session: &str,
    kind: &str,
    title: &str,
    text: &str,
    task: Option<u64>,
) -> Result<Captured, BosunError> {
    let sig = signature(kind, text);
    let on_task = task.map_or(String::new(), |t| format!(" on #{t}"));
    let existing = gh
        .list_issues(&[LESSON_LABEL], IssueState::Open)?
        .into_iter()
        .find(|i| parse_lesson_marker(i.body_text()).is_some_and(|(s, _)| s == sig));
    if let Some(issue) = existing {
        let record = Record::LessonSeen {
            session: session.into(),
            signature: sig.clone(),
            issue: task,
        };
        let comment = format!("Seen again{on_task}.\n\n{}", record.to_line());
        gh.add_comment(issue.number, &comment)?;
        let hits = notes(gh, issue.number)?
            .iter()
            .filter(|n| {
                matches!(parse_record(&n.body),
                    Some(Record::LessonSeen { signature, .. }) if signature == sig)
            })
            .count();
        return Ok(Captured {
            issue: issue.number,
            signature: sig,
            created: false,
            hits,
        });
    }
    gh.ensure_label(LESSON_LABEL, "fbca04", "a failure the curator triages")?;
    gh.ensure_label("bosun:source/lesson", "ededed", "opened by lesson capture")?;
    // Failure text must not close the fence or carry a marker of its own.
    let fence = |t: &str| t.replace("```", "'''").replace("<!--", "<!-\u{200b}-");
    let raw: Vec<&str> = text.lines().take(60).collect();
    let body = format!(
        "A failure of kind `{kind}` seen{on_task}. The curator triages it: blocking, mechanizable or advice (spec §6).\n\n```text\n{}\n```\n\nNormalised for the signature:\n\n```text\n{}\n```\n\n{}\n",
        fence(&raw.join("\n")),
        fence(&normalise(text)),
        lesson_marker(&sig, kind)
    );
    let issue = gh.create_issue(
        &format!("lesson: {title}"),
        &body,
        &[LESSON_LABEL, "bosun:source/lesson"],
    )?;
    Ok(Captured {
        issue: issue.number,
        signature: sig,
        created: true,
        hits: 0,
    })
}
