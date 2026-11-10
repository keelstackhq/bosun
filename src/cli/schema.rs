//! `bosun schema [<command>]`: the output contract as JSON Schema (spec §1.2).

use super::SchemaArgs;
use crate::output::{BosunError, CmdResult, Outcome};
use crate::schema::{data_of, envelope, names, to_json_schema};
use serde_json::{Map, Value, json};

/// A notation's JSON Schema; every notation parses (test `every_data_notation_parses`).
fn data_schema(notation: &str) -> Value {
    to_json_schema(notation).unwrap_or_default()
}

pub fn run(args: &SchemaArgs) -> CmdResult {
    if args.command.is_empty() {
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
    let name = args.command.join(" ");
    let Some(d) = data_of(&name) else {
        return Err(BosunError::usage(
            format!("no command `{name}`"),
            "run `bosun schema` for every command's schema",
        ));
    };
    Ok(Outcome::data(json!({
        "format": "json", "command": name, "schema": envelope(Some(&name), data_schema(d)),
    })))
}
