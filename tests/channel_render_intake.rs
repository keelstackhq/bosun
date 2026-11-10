//! The channel's intake seed `.bosun/intake.toml` and the `intake` var that turns on the
//! `intake = true` key in `.bosun/bosun.toml` (zero-touch onboarding, spec section 3). A minimal
//! renderer covers the template subset these two files use (`{{ var }}`, `{% if var %}`,
//! `{% if "piece" in pieces %}`, `{% else %}`, `{% endif %}`).

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

fn render(rel: &str, vars: &[(&str, &str)], pieces: &[&str]) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("channel/pieces/core/files")
        .join(rel);
    let src = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let vars: BTreeMap<&str, &str> = vars.iter().copied().collect();
    let holds = |expr: &str| match expr.split_once(" in ") {
        Some((lit, _)) => pieces.contains(&lit.trim().trim_matches('"')),
        None => !vars[expr].is_empty(),
    };
    // Each frame: (enclosing text is rendered, condition held).
    let mut stack: Vec<(bool, bool)> = vec![];
    let on = |s: &[(bool, bool)]| s.last().is_none_or(|f| f.0 && f.1);
    let (mut out, mut rest) = (String::new(), src.as_str());
    while let Some(i) = rest.find('{') {
        let (text, tag) = rest.split_at(i);
        if on(&stack) {
            out.push_str(text);
        }
        let (open, close) = if tag.starts_with("{{") {
            ("{{", "}}")
        } else if tag.starts_with("{%") {
            ("{%", "%}")
        } else {
            if on(&stack) {
                out.push('{');
            }
            rest = &tag[1..];
            continue;
        };
        let end = tag.find(close).expect("unterminated tag");
        let body = tag[2..end].trim();
        rest = &tag[end + 2..];
        if open == "{{" {
            if on(&stack) {
                out.push_str(vars[body]);
            }
        } else if body == "else" {
            let f = stack.last_mut().unwrap();
            f.1 = !f.1;
        } else if body == "endif" {
            stack.pop().unwrap();
        } else {
            let cond = holds(body.strip_prefix("if ").expect("only `if` tags"));
            stack.push((on(&stack), cond));
        }
    }
    if on(&stack) {
        out.push_str(rest);
    }
    out
}

fn vars<'a>(
    owner: &'a str,
    bosun_version: &'a str,
    intake: &'a str,
) -> Vec<(&'static str, &'a str)> {
    vec![
        ("project", "app"),
        ("owner", owner),
        ("bosun_version", bosun_version),
        ("template.version", "0.5.0"),
        ("rust_path", "."),
        ("ts_path", "."),
        ("own_ci", ""),
        ("intake", intake),
        ("policy", ""),
        ("ci_provider", ""),
    ]
}

fn config_of(bosun_version: &str, intake: &str) -> String {
    render(
        ".bosun/bosun.toml.jinja",
        &vars("", bosun_version, intake),
        &["core", "rust"],
    )
}

fn seed(owner: &str, pieces: &[&str]) -> String {
    render(
        ".bosun/intake.toml.jinja",
        &vars(owner, "", ""),
        &[&["core"], pieces].concat(),
    )
}

/// Run `bosun intake check` on `text`: `(exit code, data.field)`.
fn check(text: &str) -> (i32, Option<String>) {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("intake.toml"), text).unwrap();
    let out = common::bosun()
        .current_dir(dir.path())
        .args(["intake", "check", "intake.toml"])
        .output()
        .unwrap();
    let v = common::one_object(&out.stdout);
    let field = v["data"]["field"].as_str().map(str::to_string);
    (out.status.code().unwrap(), field)
}

#[test]
fn default_render_of_bosun_toml_has_no_intake_key() {
    for bosun_version in ["", "0.4.0"] {
        let text = config_of(bosun_version, "");
        assert!(!text.contains("intake"), "{text}");
        let t: toml::Table = text.parse().unwrap();
        assert!(!t.contains_key("intake"));
    }
    // Byte for byte what the template rendered before the var existed.
    let before = "project = \"app\"\n\n[[stack]]\npath = \".\"\nprofile = \"rust\"\n\n[review]\n";
    assert!(config_of("", "").contains(before), "{}", config_of("", ""));
}

#[test]
fn intake_key_renders_only_for_the_channel_release() {
    let on = config_of("", "1");
    let t: toml::Table = on.parse().unwrap();
    assert_eq!(t["intake"].as_bool(), Some(true), "{on}");
    // A top-level key: before the first table.
    assert!(on.find("intake = true").unwrap() < on.find("[[stack]]").unwrap());
    // The wanted-intake render differs from the default by that one line.
    assert_eq!(on.replace("intake = true\n", ""), config_of("", ""));
    // A repo pinning an explicit release may not know the key.
    for pinned in ["0.4.0", "0.5.0"] {
        assert_eq!(config_of(pinned, "1"), config_of(pinned, ""), "{pinned}");
    }
}

#[test]
fn seed_is_a_template_that_intake_check_refuses_with_exit_5() {
    for (owner, pieces, field) in [
        ("octo", &["rust"][..], "repo.visibility"),
        ("", &["rust"][..], "repo.owner"),
        ("octo", &["ts-pnpm"][..], "repo.visibility"),
    ] {
        let text = seed(owner, pieces);
        text.parse::<toml::Table>().unwrap();
        assert!(!text.contains("{{") && !text.contains("{%"), "{text}");
        assert_eq!(check(&text), (5, Some(field.to_string())), "{text}");
    }
}

#[test]
fn seed_prefills_only_what_the_vars_know() {
    let t: toml::Table = seed("octo", &["rust", "ts-pnpm"]).parse().unwrap();
    assert_eq!(t["repo"]["name"].as_str(), Some("app"));
    assert_eq!(t["repo"]["owner"].as_str(), Some("octo"));
    let stack: Vec<&str> = t["stack"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert_eq!(stack, ["rust", "ts-pnpm"]);
    // Every answer the owner still has to give is empty, never a guessed value.
    for k in ["goal", "approvals", "notify"] {
        assert_eq!(t[k].as_str(), Some(""), "{k}");
    }
}

#[test]
fn seed_passes_the_check_once_the_answers_are_filled_in() {
    let text = seed("octo", &["rust"])
        .replace("visibility = \"\"", "visibility = \"private\"")
        .replace("\npieces = []", "\npieces = [\"base\", \"rust-cli\"]")
        .replace("goal = \"\"", "goal = \"A CLI\"")
        .replace("publish = []", "publish = [\"none\"]")
        .replace("approvals = \"\"", "approvals = \"owner-gated\"")
        .replace("notify = \"\"", "notify = \"chat\"");
    assert_eq!(check(&text), (0, None), "{text}");
}

#[test]
fn seed_is_listed_in_the_core_piece_and_bosun_does_not_use_the_key() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let piece: toml::Table = fs::read_to_string(root.join("channel/pieces/core/piece.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let seed = piece["files"]["seed"].as_array().unwrap();
    assert!(
        seed.iter()
            .any(|s| s.as_str() == Some(".bosun/intake.toml"))
    );
    // Release 0.4.0 is pinned and rejects the key: bosun's own config must not set it.
    let own: toml::Table = fs::read_to_string(root.join(".bosun/bosun.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(!own.contains_key("intake"));
    let splice: toml::Table = fs::read_to_string(root.join("channel/splice.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert!(splice["vars"].as_table().unwrap().contains_key("intake"));
}
