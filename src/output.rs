//! The one JSON object every command prints (spec §1.2) and the fixed exit codes (spec §1.1).

use serde::Serialize;
use serde_json::{Value, json};

/// Exit codes, fixed for all commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    Ok,
    Error,
    Usage,
    Conflict,
    Halted,
    NeedsHuman,
    Limit,
}

impl Exit {
    pub const ALL: [Exit; 7] = [
        Exit::Ok,
        Exit::Error,
        Exit::Usage,
        Exit::Conflict,
        Exit::Halted,
        Exit::NeedsHuman,
        Exit::Limit,
    ];

    pub fn code(self) -> u8 {
        match self {
            Exit::Ok => 0,
            Exit::Error => 1,
            Exit::Usage => 2,
            Exit::Conflict => 3,
            Exit::Halted => 4,
            Exit::NeedsHuman => 5,
            Exit::Limit => 6,
        }
    }

    /// The `error.kind` value in the envelope.
    pub fn name(self) -> &'static str {
        match self {
            Exit::Ok => "ok",
            Exit::Error => "error",
            Exit::Usage => "usage",
            Exit::Conflict => "conflict",
            Exit::Halted => "halted",
            Exit::NeedsHuman => "needs-human",
            Exit::Limit => "limit",
        }
    }
}

/// A failure or refusal. `fix` always says what to do next.
#[derive(Debug, Clone, PartialEq)]
pub struct BosunError {
    pub exit: Exit,
    pub message: String,
    pub fix: String,
    /// Extra facts for the session (for example the failed spawn check).
    pub data: Option<Value>,
}

impl std::fmt::Display for BosunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}; fix: {}", self.message, self.fix)
    }
}

impl std::error::Error for BosunError {}

impl BosunError {
    pub fn new(exit: Exit, message: impl Into<String>, fix: impl Into<String>) -> Self {
        let fix = fix.into();
        debug_assert!(!fix.trim().is_empty(), "every error needs a fix");
        BosunError {
            exit,
            message: message.into(),
            fix,
            data: None,
        }
    }

    pub fn error(message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self::new(Exit::Error, message, fix)
    }

    pub fn usage(message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self::new(Exit::Usage, message, fix)
    }

    pub fn conflict(message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self::new(Exit::Conflict, message, fix)
    }

    pub fn halted(message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self::new(Exit::Halted, message, fix)
    }

    pub fn needs_human(message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self::new(Exit::NeedsHuman, message, fix)
    }

    pub fn limit(message: impl Into<String>, fix: impl Into<String>) -> Self {
        Self::new(Exit::Limit, message, fix)
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

/// A tool call the session must make (spec D29).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Next {
    pub tool: String,
    pub args: Value,
}

/// The result of a command that did its work.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Outcome {
    pub data: Value,
    pub next: Vec<Next>,
}

impl Outcome {
    pub fn data(data: Value) -> Self {
        Outcome {
            data,
            next: Vec::new(),
        }
    }

    pub fn with_next(mut self, tool: impl Into<String>, args: Value) -> Self {
        self.next.push(Next {
            tool: tool.into(),
            args,
        });
        self
    }
}

pub type CmdResult = Result<Outcome, BosunError>;

/// The exit code of a command result.
pub fn exit_of(result: &CmdResult) -> Exit {
    match result {
        Ok(_) => Exit::Ok,
        Err(e) => e.exit,
    }
}

/// The envelope: `{version, command, ok, exit_code, data, error, next}`.
pub fn envelope(command: &str, result: &CmdResult) -> Value {
    let exit = exit_of(result);
    let (data, error, next) = match result {
        Ok(o) => (
            normalize(&o.data),
            Value::Null,
            serde_json::to_value(&o.next).unwrap_or_else(|_| json!([])),
        ),
        Err(e) => (
            e.data
                .clone()
                .map(|d| normalize(&d))
                .unwrap_or_else(|| json!({})),
            json!({"kind": e.exit.name(), "message": e.message, "fix": e.fix}),
            json!([]),
        ),
    };
    json!({
        "version": 1,
        "command": command,
        "ok": exit == Exit::Ok,
        "exit_code": exit.code(),
        "data": data,
        "error": error,
        "next": next,
    })
}

/// `data` is always an object; `null` becomes `{}`.
fn normalize(data: &Value) -> Value {
    match data {
        Value::Null => json!({}),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_names_are_fixed() {
        let pairs: Vec<(u8, &str)> = Exit::ALL.iter().map(|e| (e.code(), e.name())).collect();
        assert_eq!(
            pairs,
            vec![
                (0, "ok"),
                (1, "error"),
                (2, "usage"),
                (3, "conflict"),
                (4, "halted"),
                (5, "needs-human"),
                (6, "limit"),
            ]
        );
    }

    #[test]
    fn ok_envelope_has_all_keys() {
        let r: CmdResult =
            Ok(Outcome::data(json!({"issue": 4}))
                .with_next("create_session", json!({"title": "x"})));
        let v = envelope("claim", &r);
        assert_eq!(v["version"], 1);
        assert_eq!(v["command"], "claim");
        assert_eq!(v["ok"], true);
        assert_eq!(v["exit_code"], 0);
        assert_eq!(v["data"]["issue"], 4);
        assert!(v["error"].is_null());
        assert_eq!(v["next"][0]["tool"], "create_session");
    }

    #[test]
    fn error_envelope_per_exit() {
        for exit in &Exit::ALL[1..] {
            let r: CmdResult = Err(BosunError::new(*exit, "m", "f"));
            let v = envelope("x", &r);
            assert_eq!(v["ok"], false);
            assert_eq!(v["exit_code"], exit.code());
            assert_eq!(v["error"]["kind"], exit.name());
            assert_eq!(v["error"]["fix"], "f");
            assert_eq!(v["data"], json!({}));
            assert_eq!(v["next"], json!([]));
        }
    }

    #[test]
    fn error_data_is_kept() {
        let e = BosunError::limit("cap", "wait").with_data(json!({"check": 4}));
        assert_eq!(envelope("spawn", &Err(e))["data"]["check"], 4);
    }

    #[test]
    fn null_data_becomes_object() {
        let v = envelope("x", &Ok(Outcome::default()));
        assert_eq!(v["data"], json!({}));
    }
}
