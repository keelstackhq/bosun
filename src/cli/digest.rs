//! `bosun digest`: not built yet (plan PR 4).

use crate::output::CmdResult;

pub fn run(_: Option<&std::path::Path>, _: &super::DigestArgs) -> CmdResult {
    Err(super::not_built("digest", "4"))
}
