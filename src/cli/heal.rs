//! `bosun heal`: not built yet (plan PR 3a).

use std::path::Path;

use super::HealArgs;
use crate::output::CmdResult;

pub fn run(_dir: Option<&Path>, _args: &HealArgs) -> CmdResult {
    Err(super::not_built("heal", "3a"))
}
