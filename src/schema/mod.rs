//! The output contract (spec §1.2): the envelope and the `data` shape of every command, written in
//! a small TypeScript-like notation (`data.rs`) and turned into JSON Schema. Tests check every
//! envelope they see against it, so the shapes cannot drift from the code.
//!
//! Notation: `any null bool int num str "lit"`, `[T]`, `A | B`, `{key: T, opt?: T}`; `{[str]: T}`
//! is a map and `..` in an object allows keys that are not listed. The published JSON Schema is
//! open (new keys are not a breaking change); the tests' check (`tests/common/schema_check.rs`)
//! is strict: every key is listed.

mod data;
pub mod ts;

use serde_json::{Map, Value, json};

use data::DATA;

fn entries() -> impl Iterator<Item = (&'static str, &'static str)> {
    DATA.split(";\n")
        .filter_map(|e| e.split_once(" = "))
        .map(|(n, d)| (n.trim(), d.trim()))
}

/// The command names with a schema, as the envelope's `command` names them.
pub fn names() -> impl Iterator<Item = &'static str> {
    entries().map(|(n, _)| n)
}

/// The `data` notation of a command.
pub fn data_of(command: &str) -> Option<&'static str> {
    let d = entries().find(|(n, _)| *n == command)?.1;
    Some(entries().find(|(n, _)| *n == d).map_or(d, |(_, same)| same))
}

/// A parsed notation.
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Any,
    Null,
    Bool,
    Int,
    Num,
    Str,
    Lit(String),
    Arr(Box<Ty>),
    /// Keys `(name, required, type)`; `open` allows unlisted keys.
    Obj(Vec<(String, bool, Ty)>, bool),
    Map(Box<Ty>),
    Union(Vec<Ty>),
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
            self.i += 1;
        }
    }

    fn eat(&mut self, t: &str) -> bool {
        self.ws();
        let hit = self.s[self.i..].starts_with(t.as_bytes());
        if hit {
            self.i += t.len();
        }
        hit
    }

    fn expect(&mut self, t: &str) -> Result<(), String> {
        if self.eat(t) {
            Ok(())
        } else {
            Err(format!("expected `{t}` at byte {}", self.i))
        }
    }

    fn word(&mut self) -> String {
        self.ws();
        let start = self.i;
        while self
            .s
            .get(self.i)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
        {
            self.i += 1;
        }
        String::from_utf8_lossy(&self.s[start..self.i]).into_owned()
    }

    fn ty(&mut self) -> Result<Ty, String> {
        let mut alts = vec![self.atom()?];
        while self.eat("|") {
            alts.push(self.atom()?);
        }
        Ok(if alts.len() == 1 {
            alts.remove(0)
        } else {
            Ty::Union(alts)
        })
    }

    fn atom(&mut self) -> Result<Ty, String> {
        if self.eat("[") {
            let t = self.ty()?;
            self.expect("]")?;
            return Ok(Ty::Arr(Box::new(t)));
        }
        if self.eat("\"") {
            let start = self.i;
            while self.s.get(self.i).is_some_and(|c| *c != b'"') {
                self.i += 1;
            }
            let lit = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
            self.expect("\"")?;
            return Ok(Ty::Lit(lit));
        }
        if self.eat("{") {
            return self.obj();
        }
        let at = self.i;
        Ok(match self.word().as_str() {
            "any" => Ty::Any,
            "null" => Ty::Null,
            "bool" => Ty::Bool,
            "int" => Ty::Int,
            "num" => Ty::Num,
            "str" => Ty::Str,
            w => return Err(format!("unknown type `{w}` at byte {at}")),
        })
    }

    fn obj(&mut self) -> Result<Ty, String> {
        let (mut keys, mut open) = (Vec::new(), false);
        while !self.eat("}") {
            if self.eat("..") {
                open = true;
            } else if self.eat("[str]:") {
                let t = self.ty()?;
                self.expect("}")?;
                return Ok(Ty::Map(Box::new(t)));
            } else {
                let key = self.word();
                if key.is_empty() {
                    return Err(format!("expected a key at byte {}", self.i));
                }
                let required = !self.eat("?");
                self.expect(":")?;
                keys.push((key, required, self.ty()?));
            }
            if !self.eat(",") {
                self.expect("}")?;
                break;
            }
        }
        Ok(Ty::Obj(keys, open))
    }
}

/// Parse a notation string.
pub fn parse(src: &str) -> Result<Ty, String> {
    let mut p = Parser {
        s: src.as_bytes(),
        i: 0,
    };
    let t = p.ty()?;
    p.ws();
    if p.i != src.len() {
        return Err(format!("trailing text at byte {}", p.i));
    }
    Ok(t)
}

fn json_schema(t: &Ty) -> Value {
    match t {
        Ty::Any => json!({}),
        Ty::Null => json!({"type": "null"}),
        Ty::Bool => json!({"type": "boolean"}),
        Ty::Int => json!({"type": "integer"}),
        Ty::Num => json!({"type": "number"}),
        Ty::Str => json!({"type": "string"}),
        Ty::Lit(l) => json!({"const": l}),
        Ty::Arr(i) => json!({"type": "array", "items": json_schema(i)}),
        Ty::Map(v) => json!({"type": "object", "additionalProperties": json_schema(v)}),
        Ty::Union(alts) => json!({"anyOf": alts.iter().map(json_schema).collect::<Vec<_>>()}),
        Ty::Obj(keys, _) if keys.is_empty() => json!({"type": "object"}),
        Ty::Obj(keys, _) => {
            let props: Map<String, Value> = keys
                .iter()
                .map(|(k, _, t)| (k.clone(), json_schema(t)))
                .collect();
            let req: Vec<&String> = keys.iter().filter(|k| k.1).map(|k| &k.0).collect();
            json!({"type": "object", "required": req, "properties": props})
        }
    }
}

/// The JSON Schema of a notation string.
pub fn to_json_schema(notation: &str) -> Result<Value, String> {
    parse(notation).map(|t| json_schema(&t))
}

/// The `error` of a failed envelope.
pub const ERROR: &str = r#"{kind: "error" | "usage" | "conflict" | "halted" | "needs-human" | "limit"
    | "upstream", message: str, fix: str}"#;
/// The `next` tool calls.
pub const NEXT: &str = "[{tool: str, args: any}]";

/// The envelope schema; `data` is the command's data schema (on `ok`), `command` its name.
pub fn envelope(command: Option<&str>, data: Value) -> Value {
    let mut error = to_json_schema(ERROR).expect("ERROR parses");
    error["properties"]["fix"]["minLength"] = json!(1);
    let command = command.map_or(json!({"type": "string"}), |c| json!({"const": c}));
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["version", "command", "ok", "exit_code", "data", "error", "next"],
        "properties": {
            "version": {"const": 1},
            "command": command,
            "ok": {"type": "boolean"},
            "exit_code": {"enum": [0, 1, 2, 3, 4, 5, 6]},
            "data": {"type": "object"},
            "error": {"anyOf": [{"type": "null"}, error]},
            "next": to_json_schema(NEXT).unwrap_or_default(),
        },
        "anyOf": [
            {"properties": {"ok": {"const": true}, "exit_code": {"const": 0},
                "data": data, "error": {"type": "null"}}},
            {"properties": {"ok": {"const": false}, "exit_code": {"enum": [1, 2, 3, 4, 5, 6]},
                "error": error, "next": {"maxItems": 0}}},
        ],
    })
}
