//! `bosun metrics`: not built yet (plan PR 5).

use std::path::Path;

use super::MetricsArgs;
use crate::output::CmdResult;

pub fn run(_dir: Option<&Path>, _args: &MetricsArgs) -> CmdResult {
    Err(super::not_built("metrics", "5"))
}
