//! `bosun schema [<command>] [--format json|ts]`: the output contract as JSON Schema or
//! TypeScript declarations (spec §1.2).

use super::SchemaArgs;
use crate::output::{BosunError, CmdResult, Outcome};
use crate::schema::{data_of, envelope, names, to_json_schema, ts};
use serde_json::{Map, Value, json};

/// A notation's JSON Schema; every notation parses (test `every_data_notation_parses`).
fn data_schema(notation: &str) -> Value {
    to_json_schema(notation).unwrap_or_default()
}

pub fn run(args: &SchemaArgs) -> CmdResult {
    let name = args.command.join(" ");
    if !name.is_empty() && data_of(&name).is_none() {
        let subs: Vec<&str> = names()
            .filter_map(|n| n.strip_prefix(&format!("{name} ")))
            .collect();
        let fix = if subs.is_empty() {
            "run `bosun schema` for every command's schema".to_string()
        } else {
            format!(
                "name a subcommand: `bosun schema {name} <{}>`",
                subs.join("|")
            )
        };
        return Err(BosunError::usage(format!("no command `{name}`"), fix));
    }
    if args.format == "ts" {
        let one = (!name.is_empty()).then_some(name.as_str());
        let mut data = json!({"format": "ts", "ts": ts::declarations(one)});
        if let Some(c) = one {
            data["command"] = json!(c);
        }
        return Ok(Outcome::data(data));
    }
    if name.is_empty() {
        let mut commands = Map::new();
        for name in names() {
            commands.insert(
                name.to_string(),
                data_schema(data_of(name).unwrap_or("{..}")),
            );
        }
        return Ok(Outcome::data(json!({
            "format": "json", "envelope": envelope(None, json!({"type": "object"})),
            "commands": commands,
        })));
    }
    let d = data_of(&name).unwrap_or("{..}");
    Ok(Outcome::data(json!({
        "format": "json", "command": name, "schema": envelope(Some(&name), data_schema(d)),
    })))
}
