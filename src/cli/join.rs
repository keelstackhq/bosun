//! `bosun join`: register this session in the registry, or refresh its record.

use std::path::Path;

use serde_json::json;

use super::JoinArgs;
use crate::app::{App, kill_switch};
use crate::ctx::Origin;
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry::{self, JoinSpec};

pub fn run(dir: Option<&Path>, args: &JoinArgs) -> CmdResult {
    let app = App::load(dir)?;
    kill_switch(app.gh.as_ref())?;
    let origin = match args.origin.as_deref() {
        Some(o) => Some(o.parse::<Origin>().map_err(|_| {
            BosunError::error(
                format!("--origin {o:?} is not an origin"),
                "use one of spawned, slack, local, routine",
            )
        })?),
        None => None,
    };
    let (issue, m) = registry::join(
        &app,
        JoinSpec {
            origin,
            area: args.area.clone(),
            parent: args.parent.clone(),
            depth: args.depth,
        },
    )?;
    Ok(Outcome::data(json!({
        "session": m.session,
        "origin": m.origin,
        "area": m.area,
        "depth": m.depth,
        "expires": m.expires.to_string(),
        "registry_issue": issue,
    })))
}
