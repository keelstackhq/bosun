//! `bosun digest`: not built yet (plan PR 4).

use std::path::Path;

use super::DigestArgs;
use crate::output::CmdResult;

pub fn run(_dir: Option<&Path>, _args: &DigestArgs) -> CmdResult {
    Err(super::not_built("digest", "4"))
}
