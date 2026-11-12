//! `bosun mode`: the effective mode and knobs from `.bosun/bosun.toml` (offline).

use std::path::Path;

use crate::ctx::Ctx;
use crate::output::{CmdResult, Outcome};

pub fn run(dir: Option<&Path>) -> CmdResult {
    let ctx = Ctx::load(dir)?;
    let mut data = serde_json::to_value(ctx.config.mode_effective()).unwrap_or_default();
    data["brake"] = serde_json::Value::Null;
    data["brake_note"] = "measured from plan PR 5 (bosun metrics)".into();
    Ok(Outcome::data(data))
}
