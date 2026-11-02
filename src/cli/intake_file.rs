//! `bosun intake check`: validate the intake file `.bosun/intake.toml` (spec section 3). No
//! GitHub access: the file is read locally.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::IntakeCheckArgs;
use crate::core::intake::{INTAKE_PATH, parse};
use crate::output::{BosunError, CmdResult, Outcome};

pub fn check(dir: Option<&Path>, a: &IntakeCheckArgs) -> CmdResult {
    let unread = |what: String, e: std::io::Error| {
        BosunError::usage(
            format!("cannot read {what}: {}", e.kind()),
            "pass the path of the intake file, or pipe it in with --from -",
        )
    };
    let given = a.file.clone().unwrap_or_else(|| PathBuf::from(INTAKE_PATH));
    let mut text = String::new();
    if a.from.is_some() {
        let mut stdin = std::io::stdin();
        stdin
            .read_to_string(&mut text)
            .map_err(|e| unread("stdin".into(), e))?;
    } else {
        let path = dir.map_or(given.clone(), |d| d.join(&given));
        text = std::fs::read_to_string(path).map_err(|e| match (&a.file, e.kind()) {
            (None, std::io::ErrorKind::NotFound) => BosunError::needs_human(
                format!("{INTAKE_PATH} does not exist"),
                format!("write {INTAKE_PATH} (`bosun init --intake`)"),
            ),
            _ => unread(given.display().to_string(), e),
        })?;
    }
    let i = parse(&text).map_err(|e| {
        let data = json!({"field": e.field});
        let err = if e.missing {
            BosunError::needs_human(e.message, e.fix)
        } else {
            BosunError::usage(e.message, e.fix)
        };
        err.with_data(data)
    })?;
    Ok(Outcome::data(
        serde_json::to_value(&i).expect("intake serializes"),
    ))
}
