//! `bosun heal`: not built yet (plan PR 3a).

use crate::output::CmdResult;

pub fn run(_: Option<&std::path::Path>, _: &super::HealArgs) -> CmdResult {
    Err(super::not_built("heal", "3a"))
}
