//! Renders the splice channel in `channel/` for each stack variant (rust, ts-pnpm, mixed), with and
//! without `own_ci`, and parses what comes out: every workflow as YAML, every TOML and JSON file.
//! CI has no splice, so a minimal renderer covers the template subset the channel uses (`{{ var }}`,
//! `{% if var %}`, `{% if "piece" in pieces %}`, `{% else %}`, `{% endif %}`, `{% raw %}`); any
//! other tag fails the test, so the channel cannot drift past what is checked here. No Rust YAML
//! crate is in the tree, so the YAML parse uses python3 (PyYAML) or ruby (Psych) from the runner;
//! in CI (`CI` set) a missing parser fails the test.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn channel() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("channel")
}

/// One `{% if %}` level: was the enclosing text rendered, did the condition hold, and was its
/// `{% else %}` seen.
struct Frame {
    outer: bool,
    cond: bool,
    in_else: bool,
}

fn eval(expr: &str, vars: &BTreeMap<&str, String>, pieces: &[&str], at: &str) -> bool {
    if let Some((lit, rest)) = expr.split_once(" in ") {
        assert_eq!(rest.trim(), "pieces", "{at}: unknown `in` target {expr:?}");
        let piece = lit.trim().trim_matches('"');
        // A misspelt piece would be false in every variant and drop its branch unseen.
        assert!(
            channel().join("pieces").join(piece).is_dir(),
            "{at}: no piece {piece:?}"
        );
        return pieces.contains(&piece);
    }
    let v = vars
        .get(expr)
        .unwrap_or_else(|| panic!("{at}: unknown variable {expr:?}"));
    !v.is_empty()
}

/// The channel's template subset. Panics on any tag outside it.
fn render(src: &str, vars: &BTreeMap<&str, String>, pieces: &[&str], at: &str) -> String {
    let mut out = String::new();
    let mut stack: Vec<Frame> = Vec::new();
    let active = |s: &[Frame]| s.last().is_none_or(|f| f.outer && f.cond);
    let mut rest = src;
    loop {
        let next = match (rest.find("{%"), rest.find("{{")) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        let Some(i) = next else {
            if active(&stack) {
                out.push_str(rest);
            }
            break;
        };
        if active(&stack) {
            out.push_str(&rest[..i]);
        }
        let tag = &rest[i..];
        if let Some(body) = tag.strip_prefix("{{") {
            let end = body
                .find("}}")
                .unwrap_or_else(|| panic!("{at}: unterminated {{{{"));
            let name = body[..end].trim();
            let v = vars
                .get(name)
                .unwrap_or_else(|| panic!("{at}: unknown variable {name:?}"));
            if active(&stack) {
                out.push_str(v);
            }
            rest = &body[end + 2..];
            continue;
        }
        let body = &tag[2..];
        let end = body
            .find("%}")
            .unwrap_or_else(|| panic!("{at}: unterminated {{%"));
        let stmt = body[..end].trim();
        rest = &body[end + 2..];
        match stmt {
            "raw" => {
                let close = rest
                    .find("{% endraw %}")
                    .unwrap_or_else(|| panic!("{at}: unterminated raw block"));
                if active(&stack) {
                    out.push_str(&rest[..close]);
                }
                rest = &rest[close + "{% endraw %}".len()..];
            }
            "else" => {
                let f = stack
                    .last_mut()
                    .unwrap_or_else(|| panic!("{at}: else outside if"));
                assert!(!f.in_else, "{at}: a second else in one if");
                f.in_else = true;
                f.cond = !f.cond;
            }
            "endif" => {
                stack
                    .pop()
                    .unwrap_or_else(|| panic!("{at}: endif outside if"));
            }
            s => {
                let expr = s
                    .strip_prefix("if ")
                    .unwrap_or_else(|| panic!("{at}: tag outside the channel subset: {s:?}"));
                let outer = active(&stack);
                let cond = eval(expr.trim(), vars, pieces, at);
                stack.push(Frame {
                    outer,
                    cond,
                    in_else: false,
                });
            }
        }
    }
    assert!(stack.is_empty(), "{at}: unclosed if");
    out
}

/// Every file of the chosen pieces, rendered: `(path in the consumer repo, text)`. Section
/// fragments keep their path under `sections/` of the piece.
fn render_channel(pieces: &[&str], vars: &BTreeMap<&str, String>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for piece in pieces {
        let root = channel().join("pieces").join(piece).join("files");
        let mut files = Vec::new();
        collect(&root, &mut files);
        for f in files {
            let rel = f
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let text = fs::read_to_string(&f).unwrap();
            let (name, text) = match rel.strip_suffix(".jinja") {
                Some(name) => (name.to_string(), render(&text, vars, pieces, &rel)),
                None => (rel.clone(), text),
            };
            out.push((name, text));
        }
    }
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    if !dir.is_dir() {
        return;
    }
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, out);
        } else {
            out.push(path);
        }
    }
}

const PARSE_PY: &str = r#"
import sys, yaml
bad = 0
for p in sys.argv[1:]:
    try:
        d = yaml.safe_load(open(p))
        if not isinstance(d, dict) or "jobs" not in d:
            raise ValueError("not a workflow mapping with jobs")
    except Exception as e:
        bad += 1
        print("ERR %s: %s" % (p, e))
sys.exit(1 if bad else 0)
"#;

const PARSE_RB: &str = r#"
require "yaml"
bad = 0
ARGV.each do |p|
  begin
    d = YAML.safe_load(File.read(p))
    raise "not a workflow mapping with jobs" unless d.is_a?(Hash) && d.key?("jobs")
  rescue Exception => e
    bad += 1
    puts "ERR #{p}: #{e}"
  end
end
exit(bad > 0 ? 1 : 0)
"#;

/// Parse the YAML files with the first parser the machine has. `None`: no parser.
fn parse_yaml(files: &[PathBuf]) -> Option<Result<(), String>> {
    let py = Command::new("python3").args(["-c", "import yaml"]).output();
    let cmd = if py.is_ok_and(|o| o.status.success()) {
        Some(("python3", PARSE_PY))
    } else if Command::new("ruby")
        .args(["-e", "require 'yaml'"])
        .output()
        .is_ok_and(|o| o.status.success())
    {
        Some(("ruby", PARSE_RB))
    } else {
        None
    };
    let (bin, script) = cmd?;
    let flag = if bin == "ruby" { "-e" } else { "-c" };
    let out = Command::new(bin)
        .arg(flag)
        .arg(script)
        .args(files)
        .output()
        .unwrap();
    Some(if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ))
    })
}

#[test]
fn every_variant_renders_to_files_that_parse() {
    let tmp = tempfile::tempdir().unwrap();
    let mut yaml_files = Vec::new();
    let stacks: [(&str, &[&str]); 3] = [
        ("rust", &["rust"]),
        ("ts-pnpm", &["ts-pnpm"]),
        ("mixed", &["rust", "ts-pnpm"]),
    ];
    for (variant, stack) in stacks {
        for own_ci in ["", "1"] {
            for owner in ["", "octo"] {
                let mut pieces = vec!["core"];
                pieces.extend_from_slice(stack);
                pieces.push("claude-md");
                let vars: BTreeMap<&str, String> = [
                    ("project", "app"),
                    ("owner", owner),
                    ("bosun_version", ""),
                    ("template.version", env!("CARGO_PKG_VERSION")),
                    ("rust_path", "."),
                    ("ts_path", if stack.len() > 1 { "web" } else { "." }),
                    ("own_ci", own_ci),
                ]
                .into_iter()
                .map(|(k, v)| (k, v.to_string()))
                .collect();
                let at = format!("{variant} own_ci={own_ci:?} owner={owner:?}");
                let files = render_channel(&pieces, &vars);
                let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
                for want in [
                    ".github/workflows/bosun.yml",
                    "hoist.toml",
                    ".bosun/version",
                ] {
                    assert!(names.contains(&want), "{at}: no {want}");
                }
                for (name, text) in &files {
                    let no_gh = text.replace("${{", "");
                    assert!(
                        !no_gh.contains("{{") && !no_gh.contains("{%"),
                        "{at}: {name}: template tags left"
                    );
                    if name.ends_with(".toml") {
                        text.parse::<toml::Table>()
                            .unwrap_or_else(|e| panic!("{at}: {name}: {e}"));
                    } else if name.ends_with(".json") {
                        serde_json::from_str::<serde_json::Value>(text)
                            .unwrap_or_else(|e| panic!("{at}: {name}: {e}"));
                    } else if name.ends_with(".yml") || name.ends_with(".yaml") {
                        let dir = tmp.path().join(format!("{variant}-{own_ci}-{owner}"));
                        let path = dir.join(name);
                        fs::create_dir_all(path.parent().unwrap()).unwrap();
                        fs::write(&path, text).unwrap();
                        yaml_files.push(path);
                    }
                }
                let bosun_yml = &files
                    .iter()
                    .find(|(n, _)| n == ".github/workflows/bosun.yml")
                    .unwrap()
                    .1;
                let jobs = bosun::core::checks::workflow_checks(bosun_yml);
                let aggregate = if own_ci.is_empty() {
                    "ci-ok"
                } else {
                    "bosun-ok"
                };
                assert!(jobs.iter().any(|j| j == aggregate), "{at}: {jobs:?}");
                let hoist: toml::Table = files
                    .iter()
                    .find(|(n, _)| n == "hoist.toml")
                    .unwrap()
                    .1
                    .parse()
                    .unwrap();
                let required = hoist["github"]["required_checks"].as_array().unwrap();
                assert_eq!(
                    required.iter().any(|c| c.as_str() == Some("bosun-ok")),
                    !own_ci.is_empty(),
                    "{at}: {required:?}"
                );
            }
        }
    }
    // 3 stacks x 2 own_ci x 2 owner, 6 workflows each.
    assert_eq!(yaml_files.len(), 3 * 2 * 2 * 6);
    match parse_yaml(&yaml_files) {
        Some(Ok(())) => {}
        Some(Err(e)) => panic!("rendered workflows do not parse as YAML:\n{e}"),
        None if std::env::var_os("CI").is_some() => {
            panic!("CI needs python3 with PyYAML or ruby to parse the rendered workflows")
        }
        None => eprintln!("skipped the YAML parse: no python3 with PyYAML and no ruby"),
    }
}

#[test]
fn the_renderer_refuses_tags_outside_the_subset() {
    let vars: BTreeMap<&str, String> = [("x", String::from("1"))].into_iter().collect();
    assert_eq!(
        render(
            "a{% if x %}b{% else %}c{% endif %}{% raw %}{{ y }}{% endraw %}",
            &vars,
            &[],
            "t"
        ),
        "ab{{ y }}"
    );
    for bad in [
        "{% for p in pieces %}",
        "{{ y }}",
        "{% if x %}",
        "{{ x | upper }}",
        "{% if x %}a{% else %}b{% else %}c{% endif %}",
        "{% if \"rsut\" in pieces %}{% endif %}",
    ] {
        let r = std::panic::catch_unwind(|| render(bad, &vars, &[], "t"));
        assert!(r.is_err(), "{bad} rendered");
    }
}
