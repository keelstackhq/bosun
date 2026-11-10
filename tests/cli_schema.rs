//! `bosun schema`: the envelope contract and the `data` shape of every command (spec §1.2).

mod common;

use bosun::schema;
use clap::CommandFactory;
use common::schema_check::check_envelope;
use common::{bosun, one_object};
use serde_json::{Value, json};

fn schema_cmd(args: &[&str]) -> (i32, Value) {
    let out = bosun().arg("schema").args(args).output().unwrap();
    (out.status.code().unwrap(), one_object(&out.stdout))
}

/// Every envelope command name the CLI can print: `claim`, `wave plan`, `intake`, `intake check`.
/// Hooks print the Claude Code hook protocol, not the envelope. `trailers` names its envelope
/// `trailers` for every subcommand (`Command::name`).
fn cli_names() -> Vec<String> {
    let mut names = Vec::new();
    for sub in bosun::Cli::command().get_subcommands() {
        let top = sub.get_name();
        if top == "hook" || top == "help" {
            continue;
        }
        let subs: Vec<_> = sub
            .get_subcommands()
            .filter(|s| s.get_name() != "help")
            .collect();
        if top == "trailers" {
            names.push(top.to_string());
            continue;
        }
        if subs.is_empty() || !sub.is_subcommand_required_set() {
            names.push(top.to_string());
        }
        for s in subs {
            names.push(format!("{top} {}", s.get_name()));
        }
    }
    names.sort();
    names
}

#[test]
fn every_cli_command_has_a_schema_entry() {
    let mut have: Vec<String> = schema::names().map(str::to_string).collect();
    have.sort();
    assert_eq!(have, cli_names());
    let (code, v) = schema_cmd(&[]);
    assert_eq!(code, 0, "{v}");
    let commands = v["data"]["commands"].as_object().unwrap();
    assert_eq!(commands.len(), have.len());
    assert!(commands.contains_key("verdict post"));
    assert_eq!(
        v["data"]["envelope"]["required"].as_array().unwrap().len(),
        7
    );
}

#[test]
fn one_command_prints_its_envelope_schema() {
    let (code, v) = schema_cmd(&["claim"]);
    assert_eq!(code, 0, "{v}");
    let s = &v["data"]["schema"];
    assert_eq!(s["properties"]["command"]["const"], "claim");
    let ok = &s["anyOf"][0]["properties"];
    assert_eq!(ok["data"]["properties"]["issue"]["type"], "integer");
    assert!(
        ok["data"]["required"]
            .as_array()
            .unwrap()
            .contains(&json!("lease"))
    );
    // A two-word command takes two arguments.
    let (code, v) = schema_cmd(&["verdict", "post"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["data"]["command"], "verdict post");
}

#[test]
fn an_unknown_command_is_a_usage_error() {
    let (code, v) = schema_cmd(&["nope"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["kind"], "usage");
    assert!(v["error"]["fix"].as_str().unwrap().contains("bosun schema"));
}

#[test]
fn the_schema_command_output_matches_its_own_schema() {
    for args in [vec![], vec!["claim"]] {
        let (_, v) = schema_cmd(&args);
        check_envelope(&v).unwrap();
    }
}

fn claim_sample() -> Value {
    json!({"version":1,"command":"claim","ok":true,"exit_code":0,"error":null,"next":[],
        "data":{"comment_id":7,"issue":4,"lessons":[],
                "lease":{"expires":"2026-10-22T12:00:00Z","session":"s","touch":["src/**"]}}})
}

#[test]
fn a_sample_passes_and_drift_fails() {
    check_envelope(&claim_sample()).unwrap();
    // A renamed key, a wrong type, an undocumented key, a broken envelope: all refused.
    let mut v = claim_sample();
    v["data"]["issue_no"] = v["data"]["issue"].take();
    v["data"].as_object_mut().unwrap().remove("issue");
    assert!(check_envelope(&v).unwrap_err().contains("issue"));
    let mut v = claim_sample();
    v["data"]["issue"] = json!("4");
    assert!(check_envelope(&v).is_err());
    let mut v = claim_sample();
    v["data"]["lease"]["extra"] = json!(1);
    assert!(check_envelope(&v).unwrap_err().contains("extra"));
    let mut v = claim_sample();
    v["ok"] = json!(false);
    assert!(check_envelope(&v).is_err());
    // An error envelope carries free data and a non-empty fix.
    let err = json!({"version":1,"command":"claim","ok":false,"exit_code":3,"data":{"conflicts":[]},
        "error":{"kind":"conflict","message":"m","fix":"f"},"next":[]});
    check_envelope(&err).unwrap();
    let mut bad = err.clone();
    bad["error"]["fix"] = json!("");
    assert!(check_envelope(&bad).is_err());
}

#[test]
fn the_notation_turns_into_json_schema() {
    let s = schema::to_json_schema(
        "{a: int, b?: str | null, c: [\"x\" | \"y\"], d: {[str]: num}, e: {..}}",
    )
    .unwrap();
    assert_eq!(
        s,
        json!({"type":"object","required":["a","c","d","e"],"properties":{
            "a":{"type":"integer"},
            "b":{"anyOf":[{"type":"string"},{"type":"null"}]},
            "c":{"type":"array","items":{"anyOf":[{"const":"x"},{"const":"y"}]}},
            "d":{"type":"object","additionalProperties":{"type":"number"}},
            "e":{"type":"object"}}})
    );
    assert!(schema::to_json_schema("{a: }").is_err());
}

#[test]
fn every_data_notation_parses() {
    for name in schema::names() {
        let d = schema::data_of(name).unwrap();
        schema::to_json_schema(d).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}
