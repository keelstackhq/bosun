//! `.bosun/intake.toml`: the owner's upfront answers (spec section 3, "The intake file"). Pure
//! parsing and validation; the command reads the file. A missing answer (absent or empty field)
//! is told apart from a bad value, because the first needs the owner and the second a fix.

use serde::Deserialize;
use toml::Value;

/// Where the file lives in a working tree.
pub const INTAKE_PATH: &str = ".bosun/intake.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntakeError {
    /// An answer is absent or empty: exit 5.
    Missing { field: String, fix: String },
    /// A bad value, unknown key, bad type or broken TOML: exit 2.
    Bad { field: String, why: String },
}

impl IntakeError {
    pub fn field(&self) -> &str {
        match self {
            Self::Missing { field, .. } | Self::Bad { field, .. } => field,
        }
    }
    pub fn message(&self) -> String {
        match self {
            Self::Missing { field, .. } => format!("intake answer missing: `{field}`"),
            Self::Bad { field, why } => format!("intake field `{field}`: {why}"),
        }
    }
    pub fn fix(&self) -> String {
        match self {
            Self::Missing { fix, .. } => fix.clone(),
            Self::Bad { field, why } => format!(
                "correct `{field}` in {INTAKE_PATH} ({why}); the rules are in docs/spec/03-layout.md"
            ),
        }
    }
}

fn missing(field: &str, ask: &str) -> IntakeError {
    IntakeError::Missing {
        field: field.into(),
        fix: format!("ask the owner {ask}, then set `{field}` in {INTAKE_PATH}"),
    }
}

fn bad(field: &str, why: impl Into<String>) -> IntakeError {
    IntakeError::Bad {
        field: field.into(),
        why: why.into(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repo {
    pub name: String,
    pub owner: String,
    pub visibility: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intake {
    pub repo: Repo,
    pub stack: Vec<String>,
    pub pieces: Vec<String>,
    pub bosun_pieces: Vec<String>,
    pub goal: Option<String>,
    pub plan: Option<String>,
    pub publish: Vec<String>,
    pub approvals: String,
    pub notify: String,
    pub slack_channel: String,
}

// Fields are `Value`s so that a wrong type names its field instead of failing in serde.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRepo {
    name: Option<Value>,
    owner: Option<Value>,
    visibility: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    version: Option<Value>,
    repo: Option<RawRepo>,
    stack: Option<Value>,
    pieces: Option<Value>,
    bosun_pieces: Option<Value>,
    goal: Option<Value>,
    plan: Option<Value>,
    publish: Option<Value>,
    approvals: Option<Value>,
    notify: Option<Value>,
    slack_channel: Option<Value>,
}

/// A string field: absent or empty is a missing answer, a non-string a bad type.
fn text(field: &str, v: &Option<Value>, ask: &str) -> Result<String, IntakeError> {
    match v {
        None => Err(missing(field, ask)),
        Some(Value::String(s)) if s.trim().is_empty() => Err(missing(field, ask)),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(o) => Err(bad(
            field,
            format!("expected a string, got {}", o.type_str()),
        )),
    }
}

/// A list of distinct non-empty strings; absent is the empty list.
fn list(field: &str, v: &Option<Value>) -> Result<Vec<String>, IntakeError> {
    match v {
        None => Ok(vec![]),
        Some(Value::Array(a)) => {
            let mut out: Vec<String> = vec![];
            for x in a {
                let Value::String(s) = x else {
                    return Err(bad(field, "expected a list of strings"));
                };
                if s.trim().is_empty() {
                    return Err(bad(field, "has an empty entry"));
                }
                if out.contains(s) {
                    return Err(bad(field, format!("{s:?} is listed twice")));
                }
                out.push(s.clone());
            }
            Ok(out)
        }
        Some(o) => Err(bad(field, format!("expected a list, got {}", o.type_str()))),
    }
}

fn one_of(field: &str, v: &str, allowed: &[&str]) -> Result<(), IntakeError> {
    if allowed.contains(&v) {
        Ok(())
    } else {
        Err(bad(
            field,
            format!("{v:?} is not one of {}", allowed.join(", ")),
        ))
    }
}

fn name_ok(s: &str) -> bool {
    !s.starts_with('.')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn plan_ok(p: &str) -> bool {
    p.starts_with("plans/")
        && p.ends_with(".md")
        && p.split('/').all(|x| {
            !matches!(x, "" | "." | "..")
                && x.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        })
}

const TOP: [&str; 11] = [
    "version",
    "repo",
    "stack",
    "pieces",
    "bosun_pieces",
    "goal",
    "plan",
    "publish",
    "approvals",
    "notify",
    "slack_channel",
];
const REPO: [&str; 3] = ["name", "owner", "visibility"];

/// Read the TOML and check its shape, so that every structural error names its key.
fn structure(src: &str) -> Result<Raw, IntakeError> {
    let table: toml::Table = src.parse().map_err(|e: toml::de::Error| {
        // A duplicate key has the key as its span; a syntax error has no key to name.
        let key = e.span().map_or("", |r| src[r].trim());
        let named = !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        bad(if named { key } else { "(file)" }, e.message().to_string())
    })?;
    if let Some(k) = table.keys().find(|k| !TOP.contains(&k.as_str())) {
        return Err(bad(k, "unknown key"));
    }
    match table.get("repo") {
        Some(Value::Table(t)) => {
            if let Some(k) = t.keys().find(|k| !REPO.contains(&k.as_str())) {
                return Err(bad(&format!("repo.{k}"), "unknown key"));
            }
        }
        Some(_) => {
            return Err(bad(
                "repo",
                "expected a table with name, owner and visibility",
            ));
        }
        None => {}
    }
    Value::Table(table)
        .try_into()
        .map_err(|_| bad("(file)", "the file does not have the shape of an intake"))
}

/// Parse and validate. Fields are checked in file order, so the first error is the first thing
/// the owner has to fix.
pub fn parse(src: &str) -> Result<Intake, IntakeError> {
    let raw = structure(src)?;
    match &raw.version {
        None => return Err(missing("version", "nothing: set `version = 1`")),
        Some(Value::Integer(1)) => {}
        Some(_) => return Err(bad("version", "must be the integer 1")),
    }
    let r = raw
        .repo
        .as_ref()
        .ok_or_else(|| missing("repo.name", "for the repo name"))?;
    let repo = Repo {
        name: text("repo.name", &r.name, "for the repo name")?,
        owner: text("repo.owner", &r.owner, "for the GitHub org (keelstackhq)")?,
        visibility: text("repo.visibility", &r.visibility, "private or public")?,
    };
    for (f, v) in [("repo.name", &repo.name), ("repo.owner", &repo.owner)] {
        if !name_ok(v) {
            return Err(bad(f, "use letters, digits, `-`, `_` and `.` only"));
        }
    }
    one_of("repo.visibility", &repo.visibility, &["private", "public"])?;

    let stack = list("stack", &raw.stack)?;
    if stack.is_empty() {
        return Err(missing("stack", "for the stack (rust or ts-pnpm)"));
    }
    for s in &stack {
        one_of("stack", s, &["rust", "ts-pnpm"])?;
    }
    let pieces = list("pieces", &raw.pieces)?;
    if pieces.is_empty() {
        return Err(missing("pieces", "which hull pieces to use"));
    }
    let piece_char = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
    if let Some(p) = pieces.iter().find(|p| !p.chars().all(piece_char)) {
        return Err(bad(
            "pieces",
            format!("{p:?} is not a piece name (lowercase, digits, `-`)"),
        ));
    }
    let bosun_pieces = list("bosun_pieces", &raw.bosun_pieces)?;
    for p in &bosun_pieces {
        one_of(
            "bosun_pieces",
            p,
            &["claude-md", "circleci", "circleci-only"],
        )?;
    }
    for (f, v) in [("goal", &raw.goal), ("plan", &raw.plan)] {
        if matches!(v, Some(x) if !x.is_str()) {
            return Err(bad(f, "expected a string"));
        }
    }
    let given = |v: &Option<Value>| match v {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        _ => None,
    };
    let (goal, plan) = (given(&raw.goal), given(&raw.plan));
    match (&goal, &plan) {
        (Some(_), Some(_)) => return Err(bad("goal", "set either `goal` or `plan`, not both")),
        (None, None) => {
            return Err(missing(
                "goal",
                "what the repo should do (`goal`), or the path of a plan (`plan`)",
            ));
        }
        _ => {}
    }
    if let Some(p) = &plan
        && !plan_ok(p)
    {
        return Err(bad("plan", "must be a path under plans/ that ends in .md"));
    }
    let publish = list("publish", &raw.publish)?;
    if publish.is_empty() {
        return Err(missing(
            "publish",
            "where it publishes (npm, crates, cloudflare, none)",
        ));
    }
    for p in &publish {
        one_of("publish", p, &["npm", "crates", "cloudflare", "none"])?;
    }
    if publish.len() > 1 && publish.iter().any(|p| p == "none") {
        return Err(bad("publish", "\"none\" stands alone"));
    }
    let approvals = text("approvals", &raw.approvals, "who approves gated PRs")?;
    one_of(
        "approvals",
        &approvals,
        &["owner", "owner-gated", "orchestrator"],
    )?;
    let notify = text("notify", &raw.notify, "how agents report (chat or slack)")?;
    one_of("notify", &notify, &["chat", "slack"])?;
    let slack_channel = match &raw.slack_channel {
        None => String::new(),
        Some(Value::String(s)) => s.trim().to_string(),
        Some(_) => return Err(bad("slack_channel", "expected a string")),
    };
    if notify == "slack" && slack_channel.is_empty() {
        return Err(missing("slack_channel", "for the Slack channel"));
    }
    if notify == "chat" && !slack_channel.is_empty() {
        return Err(bad("slack_channel", "is set but notify is \"chat\""));
    }
    if slack_channel.chars().any(char::is_whitespace) {
        return Err(bad("slack_channel", "has whitespace"));
    }
    Ok(Intake {
        repo,
        stack,
        pieces,
        bosun_pieces,
        goal,
        plan,
        publish,
        approvals,
        notify,
        slack_channel,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &str = r#"version = 1
repo = { name = "app", owner = "keelstackhq", visibility = "private" }
stack = ["rust"]
pieces = ["base", "rust-cli"]
bosun_pieces = ["claude-md"]
goal = "A CLI that does a thing"
publish = ["crates"]
approvals = "orchestrator"
notify = "chat"
slack_channel = ""
"#;

    fn edit(from: &str, to: &str) -> String {
        assert!(OK.contains(from), "{from}");
        OK.replacen(from, to, 1)
    }

    fn assert_missing(src: &str, field: &str) {
        match parse(src).unwrap_err() {
            IntakeError::Missing { field: f, fix } => {
                assert_eq!(f, field);
                assert!(!fix.trim().is_empty());
            }
            e => panic!("want missing {field}, got {e:?}"),
        }
    }

    fn assert_bad(src: &str, field: &str) {
        let e = parse(src).unwrap_err();
        assert!(
            matches!(&e, IntakeError::Bad { field: f, .. } if f == field),
            "{e:?}"
        );
        assert!(e.fix().contains(field) && e.message().contains(field));
    }

    #[test]
    fn a_complete_file_parses() {
        let p = parse(OK).unwrap();
        assert_eq!(p.repo.name, "app");
        assert_eq!(p.stack, ["rust"]);
        assert_eq!(p.goal.as_deref(), Some("A CLI that does a thing"));
    }

    #[test]
    fn absent_or_empty_answers_are_missing() {
        assert_missing(&edit("version = 1\n", ""), "version");
        assert_missing(&edit("name = \"app\"", "name = \"\""), "repo.name");
        assert_missing(
            &edit("owner = \"keelstackhq\"", "owner = \" \""),
            "repo.owner",
        );
        assert_missing(&edit(", visibility = \"private\"", ""), "repo.visibility");
        assert_missing(&edit("stack = [\"rust\"]", "stack = []"), "stack");
        assert_missing(&edit("stack = [\"rust\"]\n", ""), "stack");
        assert_missing(&edit("[\"base\", \"rust-cli\"]", "[]"), "pieces");
        assert_missing(&edit("goal = \"A CLI that does a thing\"\n", ""), "goal");
        assert_missing(&edit("A CLI that does a thing", ""), "goal");
        assert_missing(&edit("publish = [\"crates\"]\n", ""), "publish");
        assert_missing(&edit("approvals = \"orchestrator\"\n", ""), "approvals");
        assert_missing(&edit("notify = \"chat\"\n", ""), "notify");
        assert_missing(&edit("\"chat\"", "\"slack\""), "slack_channel");
    }

    #[test]
    fn bad_values_and_types_are_bad() {
        assert_bad(&edit("version = 1", "version = 2"), "version");
        assert_bad(&edit("\"private\"", "\"secret\""), "repo.visibility");
        assert_bad(&edit("name = \"app\"", "name = \"a b\""), "repo.name");
        assert_bad(&edit("name = \"app\"", "name = 3"), "repo.name");
        assert_bad(&edit("[\"rust\"]", "[\"go\"]"), "stack");
        assert_bad(&edit("[\"rust\"]", "\"rust\""), "stack");
        assert_bad(&edit("[\"rust\"]", "[\"rust\", \"rust\"]"), "stack");
        assert_bad(&edit("\"base\"", "\"Base!\""), "pieces");
        assert_bad(&edit("\"claude-md\"", "\"other\""), "bosun_pieces");
        assert_bad(&edit("[\"crates\"]", "[\"pypi\"]"), "publish");
        assert_bad(&edit("[\"crates\"]", "[\"crates\", \"none\"]"), "publish");
        assert_bad(&edit("\"orchestrator\"", "\"nobody\""), "approvals");
        assert_bad(&edit("\"chat\"", "\"email\""), "notify");
        assert_bad(
            &edit("slack_channel = \"\"", "slack_channel = \"x\""),
            "slack_channel",
        );
        assert_bad(&edit("\"A CLI that does a thing\"", "5"), "goal");
        assert_bad("version = [", "(file)");
    }

    #[test]
    fn publish_none_stands_alone() {
        let ok = edit("[\"crates\"]", "[\"none\"]");
        assert_eq!(parse(&ok).unwrap().publish, ["none"]);
    }

    #[test]
    fn goal_and_plan_exclude_each_other() {
        assert_bad(&edit("publish", "plan = \"plans/app.md\"\npublish"), "goal");
        let g = "goal = \"A CLI that does a thing\"";
        let plan = edit(g, "plan = \"plans/app.md\"");
        assert_eq!(parse(&plan).unwrap().plan.as_deref(), Some("plans/app.md"));
        for p in [
            "docs/app.md",
            "plans/app.txt",
            "plans/../x.md",
            "/plans/a.md",
        ] {
            assert_bad(&edit(g, &format!("plan = {p:?}")), "plan");
        }
    }

    #[test]
    fn slack_with_a_channel_parses() {
        let ok = edit(
            "notify = \"chat\"\nslack_channel = \"\"",
            "notify = \"slack\"\nslack_channel = \"eng-bots\"",
        );
        assert_eq!(parse(&ok).unwrap().slack_channel, "eng-bots");
    }

    #[test]
    fn unknown_keys_are_bad_at_every_level() {
        assert_bad(&format!("{OK}extra = 1\n"), "extra");
        assert_bad(
            &edit(
                "visibility = \"private\"",
                "visibility = \"private\", x = 1",
            ),
            "repo.x",
        );
        // The signature is a detached file: a table in the intake is an unknown key.
        assert_bad(&format!("{OK}[signature]\nkey = \"k\"\n"), "signature");
    }

    #[test]
    fn plan_paths_allow_plain_segments_only() {
        let g = "goal = \"A CLI that does a thing\"";
        for p in [
            "plans/a\nb.md",
            "plans/ .md",
            "plans/a b.md",
            "plans/a\tb.md",
            "plans/é.md",
        ] {
            assert_bad(&edit(g, &format!("plan = {p:?}")), "plan");
        }
        let ok = edit(g, "plan = \"plans/sub/App_1.v2-x.md\"");
        assert!(parse(&ok).is_ok());
    }

    #[test]
    fn structural_errors_name_the_key_and_leak_no_serde_text() {
        let top = format!("{OK}extra = 1\n");
        let nested = edit(
            "visibility = \"private\"",
            "visibility = \"private\", x = 1",
        );
        let dup = format!("{OK}stack = [\"rust\"]\n");
        let wrong = edit(
            "repo = { name = \"app\", owner = \"keelstackhq\", visibility = \"private\" }",
            "repo = \"x\"",
        );
        let cases = [
            (top, "extra"),
            (nested, "repo.x"),
            (dup, "stack"),
            (wrong, "repo"),
        ];
        for (src, field) in cases {
            let e = parse(&src).unwrap_err();
            assert_eq!(e.field(), field, "{e:?}");
            let text = format!("{} {}", e.message(), e.fix());
            for leak in ["Raw", "struct", "expected struct"] {
                assert!(!text.contains(leak), "{leak}: {text}");
            }
        }
    }
}
