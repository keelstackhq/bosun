//! `bosun intake check`: validate the intake file `.bosun/intake.toml` (spec section 3). No
//! GitHub access: the file is read locally.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::IntakeCheckArgs;
use crate::core::intake::{INTAKE_PATH, IntakeError, parse};
use crate::output::{BosunError, CmdResult, Outcome};

pub fn check(dir: Option<&Path>, a: &IntakeCheckArgs) -> CmdResult {
    let usage = |what: &str, e: std::io::Error| {
        BosunError::usage(
            format!("cannot read {what}: {}", e.kind()),
            "pass the path of the intake file, or pipe it in with --from -",
        )
    };
    let mut text = String::new();
    if a.from.is_some() {
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(|e| usage("stdin", e))?;
    } else {
        let given = a.file.clone().unwrap_or_else(|| PathBuf::from(INTAKE_PATH));
        let path = dir.map_or(given.clone(), |d| d.join(&given));
        text = std::fs::read_to_string(path).map_err(|e| usage(&given.display().to_string(), e))?;
    }
    let i = parse(&text).map_err(|e| {
        let data = json!({"field": e.field()});
        let (m, f) = (e.message(), e.fix());
        match e {
            IntakeError::Missing { .. } => BosunError::needs_human(m, f),
            IntakeError::Bad { .. } => BosunError::usage(m, f),
        }
        .with_data(data)
    })?;
    Ok(Outcome::data(
        json!({"repo": {"name": i.repo.name, "owner": i.repo.owner, "visibility": i.repo.visibility},
            "stack": i.stack, "pieces": i.pieces, "bosun_pieces": i.bosun_pieces,
            "goal": i.goal, "plan": i.plan, "publish": i.publish, "approvals": i.approvals,
            "notify": i.notify, "slack_channel": i.slack_channel}),
    ))
}
