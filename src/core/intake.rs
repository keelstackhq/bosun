//! `.bosun/intake.toml`: the owner's upfront answers (spec section 3). Pure parsing; a missing
//! answer (absent or empty field, exit 5) is told apart from a bad value (exit 2).

use toml::Value;

/// Where the file lives in a working tree.
pub const INTAKE_PATH: &str = ".bosun/intake.toml";

/// A refused file: `missing` (an absent or empty answer, exit 5) or a bad value, unknown key,
/// bad type or broken TOML (exit 2).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct IntakeError {
    pub missing: bool,
    pub field: String,
    pub message: String,
    pub fix: String,
}

fn missing(field: &str, ask: &str) -> IntakeError {
    IntakeError {
        missing: true,
        field: field.into(),
        message: format!("intake answer missing: `{field}`"),
        fix: format!("ask the owner {ask}, then set `{field}` in {INTAKE_PATH}"),
    }
}

fn bad(field: &str, why: impl Into<String>) -> IntakeError {
    let why = why.into();
    IntakeError {
        missing: false,
        field: field.into(),
        message: format!("intake field `{field}`: {why}"),
        fix: format!("correct `{field}` in {INTAKE_PATH} ({why}); rules: docs/spec/03-layout.md"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Repo {
    pub name: String,
    pub owner: String,
    pub visibility: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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

type V<'a> = Option<&'a Value>;
type R<T> = Result<T, IntakeError>;

/// A string field: absent or empty is a missing answer, a non-string a bad type.
fn text(field: &str, v: V<'_>, ask: &str) -> R<String> {
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

/// One of `allowed` (any string when `allowed` is empty).
fn choice(field: &str, v: V, ask: &str, allowed: &[&str]) -> R<String> {
    let s = text(field, v, ask)?;
    match allowed.is_empty() || allowed.contains(&s.as_str()) {
        true => Ok(s),
        false => Err(bad(
            field,
            format!("{s:?} is not one of {}", allowed.join(", ")),
        )),
    }
}

/// A list of distinct strings; with `allowed` each must be one of them, with an `ask` it must
/// not be empty.
fn choices(field: &str, v: V, ask: &str, allowed: &[&str]) -> R<Vec<String>> {
    let items = match v {
        None => &[][..],
        Some(Value::Array(a)) => a.as_slice(),
        Some(o) => return Err(bad(field, format!("expected a list, got {}", o.type_str()))),
    };
    let mut out: Vec<String> = vec![];
    for x in items {
        let s = choice(field, Some(x), ask, allowed)?;
        if out.contains(&s) {
            return Err(bad(field, format!("{s:?} is listed twice")));
        }
        out.push(s);
    }
    match out.is_empty() && !ask.is_empty() {
        true => Err(missing(field, ask)),
        false => Ok(out),
    }
}

fn plain(s: &str, extra: &[char]) -> bool {
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
}

fn plan_ok(p: &str) -> bool {
    p.starts_with("plans/")
        && p.ends_with(".md")
        && p.split('/')
            .all(|x| !matches!(x, "" | "." | "..") && plain(x, &['.', '_', '-']))
}

const TOP: &str =
    "version repo stack pieces bosun_pieces goal plan publish approvals notify slack_channel";
const REPO: &str = "name owner visibility";

/// Read the TOML and check its shape, so that every structural error names its key.
fn structure(src: &str) -> Result<toml::Table, IntakeError> {
    let table: toml::Table = src.parse().map_err(|e: toml::de::Error| {
        // A duplicate key has the key as its span; a syntax error has no key to name.
        let key = e.span().map_or("", |r| src[r].trim());
        let named = !key.is_empty() && plain(key, &['_']);
        bad(if named { key } else { "(file)" }, e.message().to_string())
    })?;
    if let Some(k) = table.keys().find(|k| !TOP.split(' ').any(|t| t == *k)) {
        return Err(bad(k, "unknown key"));
    }
    match table.get("repo") {
        Some(Value::Table(t)) => match t.keys().find(|k| !REPO.split(' ').any(|r| r == *k)) {
            Some(k) => Err(bad(&format!("repo.{k}"), "unknown key")),
            None => Ok(table),
        },
        Some(_) => Err(bad(
            "repo",
            "expected a table with name, owner and visibility",
        )),
        None => Ok(table),
    }
}

/// Parse and validate. Fields are checked in file order, so the first error is the first thing
/// the owner has to fix.
pub fn parse(src: &str) -> Result<Intake, IntakeError> {
    let t = structure(src)?;
    match t.get("version") {
        None => return Err(missing("version", "nothing: set `version = 1`")),
        Some(Value::Integer(1)) => {}
        Some(_) => return Err(bad("version", "must be the integer 1")),
    }
    let empty = toml::Table::new();
    let r = t.get("repo").and_then(Value::as_table).unwrap_or(&empty);
    let repo = Repo {
        name: text("repo.name", r.get("name"), "for the repo name")?,
        owner: text(
            "repo.owner",
            r.get("owner"),
            "for the GitHub org (keelstackhq)",
        )?,
        visibility: choice(
            "repo.visibility",
            r.get("visibility"),
            "private or public",
            &["private", "public"],
        )?,
    };
    for (f, v) in [("repo.name", &repo.name), ("repo.owner", &repo.owner)] {
        if v.starts_with('.') || !plain(v, &['-', '_', '.']) {
            return Err(bad(f, "use letters, digits, `-`, `_` and `.` only"));
        }
    }
    let stack = choices(
        "stack",
        t.get("stack"),
        "for the stack (rust or ts-pnpm)",
        &["rust", "ts-pnpm"],
    )?;
    let pieces = choices("pieces", t.get("pieces"), "which hull pieces to use", &[])?;
    let piece = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
    if let Some(p) = pieces.iter().find(|p| !p.chars().all(piece)) {
        return Err(bad(
            "pieces",
            format!("{p:?} is not a piece name (lowercase, digits, `-`)"),
        ));
    }
    let bosun_pieces = choices(
        "bosun_pieces",
        t.get("bosun_pieces"),
        "",
        &["claude-md", "circleci", "circleci-only"],
    )?;
    for f in ["goal", "plan"] {
        if matches!(t.get(f), Some(x) if !x.is_str()) {
            return Err(bad(f, "expected a string"));
        }
    }
    let given = |k| {
        t.get(k)
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
    };
    let (goal, plan) = (given("goal"), given("plan"));
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
    if plan.as_deref().is_some_and(|p| !plan_ok(p)) {
        return Err(bad("plan", "must be a path under plans/ that ends in .md"));
    }
    let all = ["npm", "crates", "cloudflare", "none"];
    let publish = choices(
        "publish",
        t.get("publish"),
        "where it publishes (npm, crates, cloudflare, none)",
        &all,
    )?;
    if publish.len() > 1 && publish.iter().any(|p| p == "none") {
        return Err(bad("publish", "\"none\" stands alone"));
    }
    let a = ["owner", "owner-gated", "orchestrator"];
    let approvals = choice(
        "approvals",
        t.get("approvals"),
        "who approves gated PRs",
        &a,
    )?;
    let notify = choice(
        "notify",
        t.get("notify"),
        "how agents report (chat or slack)",
        &["chat", "slack"],
    )?;
    let slack_channel = match t.get("slack_channel") {
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

/// The detached signature file next to the intake (`.bosun/intake.sig`).
pub const SIG_PATH: &str = ".bosun/intake.sig";

/// What `bosun intake sign` prints and `bosun intake verify` reads.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SigFile {
    pub key: String,
    pub repo: String,
    pub mode: String,
    pub nonce: String,
    pub expires: String,
    pub intake_sha256: String,
    pub sig: String,
}

impl SigFile {
    pub fn parse(src: &str) -> Result<SigFile, String> {
        let f: SigFile = toml::from_str(src).map_err(|e| e.message().to_string())?;
        if !matches!(f.mode.as_str(), "new" | "adopt") {
            return Err(format!("mode {:?} is not new or adopt", f.mode));
        }
        Ok(f)
    }

    pub fn to_toml(&self) -> String {
        format!(
            "key = {:?}\nrepo = {:?}\nmode = {:?}\nnonce = {:?}\nexpires = {:?}\nintake_sha256 = {:?}\nsig = {:?}\n",
            self.key, self.repo, self.mode, self.nonce, self.expires, self.intake_sha256, self.sig
        )
    }

    /// The exact bytes the key signs.
    pub fn message(&self) -> String {
        signed_message(
            &self.repo,
            &self.mode,
            &self.nonce,
            &self.expires,
            &self.intake_sha256,
        )
    }
}

/// Lowercase hex SHA-256 of the exact intake bytes.
pub fn intake_sha256(bytes: &[u8]) -> String {
    let d = ring::digest::digest(&ring::digest::SHA256, bytes);
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// `bosun-intake v1 <owner/name lowercased> <mode> <nonce> <expires> sha256 <hex>`.
pub fn signed_message(repo: &str, mode: &str, nonce: &str, expires: &str, sha: &str) -> String {
    format!(
        "bosun-intake v1 {} {mode} {nonce} {expires} sha256 {sha}",
        repo.to_ascii_lowercase()
    )
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
        let e = parse(src).unwrap_err();
        assert!(
            e.missing && e.field == field && !e.fix.trim().is_empty(),
            "{e:?}"
        );
    }

    fn assert_bad(src: &str, field: &str) {
        let e = parse(src).unwrap_err();
        assert!(!e.missing && e.field == field, "{e:?}");
        assert!(e.fix.contains(field) && e.message.contains(field));
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
            assert_eq!(e.field, field, "{e:?}");
            let text = format!("{} {}", e.message, e.fix);
            for leak in ["Raw", "struct", "expected struct"] {
                assert!(!text.contains(leak), "{leak}: {text}");
            }
        }
    }

    #[test]
    fn the_sig_file_round_trips_and_binds_every_field() {
        let sha = intake_sha256(OK.as_bytes());
        assert_eq!(sha.len(), 64);
        let f = SigFile {
            key: "k1".into(),
            repo: "KeelstackHQ/App".into(),
            mode: "new".into(),
            nonce: "ab".repeat(16),
            expires: "2026-11-02T11:00:00Z".into(),
            intake_sha256: sha.clone(),
            sig: "c2ln".into(),
        };
        assert_eq!(SigFile::parse(&f.to_toml()).unwrap(), f);
        let m = f.message();
        assert!(m.starts_with("bosun-intake v1 keelstackhq/app new "));
        assert!(m.ends_with(&format!("sha256 {sha}")));
        let adopt = SigFile {
            mode: "adopt".into(),
            ..f.clone()
        };
        assert_ne!(m, adopt.message());
        assert!(SigFile::parse(&f.to_toml().replace("\"new\"", "\"x\"")).is_err());
        assert!(SigFile::parse(&format!("{}z = 1\n", f.to_toml())).is_err());
    }
}
