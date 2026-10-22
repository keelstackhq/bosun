//! `bosun lesson add|hit`: the lessons store `docs/lessons.toml` (spec §6). `add` and `hit`
//! work on the local file; `hit --issue` also posts a record, which needs GitHub.

use std::path::Path;

use serde_json::json;

use super::{LessonAddArgs, LessonCmd, LessonHitArgs};
use crate::app::{App, kill_switch};
use crate::core::lessons::{self, Store};
use crate::core::records::Record;
use crate::ctx::Ctx;
use crate::output::{BosunError, CmdResult, Outcome};

pub const FILE: &str = "docs/lessons.toml";
const HEADER: &str = "# bosun lessons store, spec §6\n\n";

pub fn run(dir: Option<&Path>, cmd: &LessonCmd) -> CmdResult {
    match cmd {
        LessonCmd::Add(a) => add(dir, a),
        LessonCmd::Hit(a) => hit(dir, a),
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
fn save(root: &Path, store: &Store) -> Result<(), BosunError> {
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

fn today(ctx: &Ctx) -> String {
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
    .map_err(|cap| {
        let (n, what) = if cap.global {
            (lessons::MAX_GLOBAL, "global lessons")
        } else {
            (lessons::MAX, "lessons")
        };
        BosunError::limit(
            format!("{FILE} already has {n} {what}"),
            "mechanize, merge or retire a lesson (spec §6)",
        )
    })?;
    save(&ctx.dir, &store)?;
    Ok(Outcome::data(
        json!({"id": id, "count": store.lesson.len(), "global_count": store.global_count()}),
    ))
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
