//! `bosun metrics`: not built yet (plan PR 5).

use crate::output::CmdResult;

pub fn run(_: Option<&std::path::Path>, _: &super::MetricsArgs) -> CmdResult {
    Err(super::not_built("metrics", "5"))
}
