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

/// One variant of the channel, rendered: `(path, text)`.
fn variant(stack: &[&str], own_ci: &str) -> Vec<(String, String)> {
    let mut pieces = vec!["core"];
    pieces.extend_from_slice(stack);
    let vars: BTreeMap<&str, String> = [
        ("project", "app"),
        ("owner", ""),
        ("bosun_version", ""),
        ("template.version", "1.2.3"),
        ("rust_path", "."),
        ("ts_path", "."),
        ("own_ci", own_ci),
    ]
    .into_iter()
    .map(|(k, v)| (k, v.to_string()))
    .collect();
    render_channel(&pieces, &vars)
}

fn file<'a>(files: &'a [(String, String)], name: &str) -> &'a str {
    &files.iter().find(|(n, _)| n == name).unwrap().1
}

/// The workflow as JSON through PyYAML; `None` without it (a failure in CI).
fn yaml_json(text: &str) -> Option<serde_json::Value> {
    let child = Command::new("python3")
        .args([
            "-c",
            "import sys, yaml, json; print(json.dumps(yaml.safe_load(sys.stdin)))",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let out = child.ok().and_then(|mut c| {
        std::io::Write::write_all(&mut c.stdin.take().unwrap(), text.as_bytes()).ok()?;
        c.wait_with_output().ok()
    });
    match out {
        Some(o) if o.status.success() => Some(serde_json::from_slice(&o.stdout).unwrap()),
        _ => {
            assert!(
                std::env::var_os("CI").is_none(),
                "CI needs python3 with PyYAML"
            );
            None
        }
    }
}

/// In run order: the gates that run no PR code first, then lint (build.rs, proc macros) and test,
/// so PR code on the one runner cannot change what test-integrity, deps or fmt report.
const ALL_GATES: [&str; 5] = ["test-integrity", "deps", "fmt", "lint", "test"];
const BOSUN_YML: &str = ".github/workflows/bosun.yml";

#[test]
fn rust_gates_install_the_toolchain_file_once() {
    for own_ci in ["", "1"] {
        let files = variant(&["rust"], own_ci);
        let wf = file(&files, BOSUN_YML);
        assert!(!wf.contains("dtolnay/rust-toolchain"), "{wf}");
        assert!(!wf.contains("\"1.97\""), "{wf}");
        assert!(wf.contains("rustup component add rustfmt clippy"), "{wf}");
        assert!(wf.contains("cache-bin: \"false\""), "{wf}");
        // Without a toolchain file in rust_path or a parent, the old pin, never `stable`; the
        // gates and the setup script fall back to the same pin.
        let setup = file(&files, "docs/setup-script.sh");
        let pin = setup
            .lines()
            .find_map(|l| l.strip_prefix("RUST_VERSION="))
            .and_then(|l| l.split_whitespace().next())
            .unwrap();
        assert!(
            wf.contains(&format!(
                "rustup toolchain install {pin} --profile minimal -c rustfmt,clippy"
            )),
            "{wf}"
        );
        assert!(wf.contains(&format!("rustup default {pin}")), "{wf}");
        assert!(!wf.contains("install stable"), "{wf}");
        assert!(setup.contains("rust-toolchain.toml"), "{setup}");
        assert!(
            setup.contains("RUST_VERSION=1.97"),
            "the fallback pin stays"
        );
    }
    let files = variant(&["ts-pnpm"], "");
    assert!(!file(&files, BOSUN_YML).contains("rustup"));
}

/// The steps of the one job, `ci-ok` (or `bosun-ok` beside the repo's own ci).
fn steps(j: &serde_json::Value, own_ci: &str) -> Vec<serde_json::Value> {
    let name = if own_ci.is_empty() {
        "ci-ok"
    } else {
        "bosun-ok"
    };
    j["jobs"][name]["steps"].as_array().unwrap().clone()
}

fn step<'a>(steps: &'a [serde_json::Value], key: &str, value: &str) -> &'a serde_json::Value {
    steps
        .iter()
        .find(|s| s[key] == value)
        .unwrap_or_else(|| panic!("no step with {key} {value}"))
}

#[test]
fn own_ci_gates_are_test_integrity_unless_opted_in() {
    for stack in [&["rust"][..], &["ts-pnpm"], &["rust", "ts-pnpm"]] {
        let plain = variant(stack, "");
        let wf = file(&plain, BOSUN_YML);
        assert!(!wf.contains("BOSUN_ALL_GATES"), "{wf}");
        let own = variant(stack, "1");
        let wf = file(&own, BOSUN_YML);
        assert!(wf.contains("vars.BOSUN_ALL_GATES == '1'"), "{wf}");
        let Some(j) = yaml_json(wf) else { return };
        assert_eq!(j["jobs"].as_object().unwrap().len(), 1, "one job");
        let s = steps(&j, "1");
        let gates = &step(&s, "name", "bosun gates")["env"]["GATES"];
        let expr = gates.as_str().unwrap();
        assert!(
            expr.contains("github.event_name != 'pull_request'"),
            "{expr}"
        );
        assert!(expr.contains("|| 'test-integrity' }}"), "{expr}");
        assert!(
            expr.contains(&format!("'{}'", ALL_GATES.join(" "))),
            "{expr}"
        );
        let j = yaml_json(file(&plain, BOSUN_YML)).unwrap();
        let s = steps(&j, "");
        let gates = &step(&s, "name", "bosun gates")["env"]["GATES"];
        assert_eq!(gates.as_str(), Some(ALL_GATES.join(" ").as_str()));
    }
}

#[test]
fn every_work_step_waits_for_the_plan() {
    for stack in [&["rust"][..], &["ts-pnpm"], &["rust", "ts-pnpm"]] {
        for own_ci in ["", "1"] {
            let files = variant(stack, own_ci);
            let Some(j) = yaml_json(file(&files, BOSUN_YML)) else {
                return;
            };
            let steps = steps(&j, own_ci);
            let plan = steps.iter().position(|s| s["id"] == "plan").unwrap();
            assert!(plan > 0 && plan < steps.len() - 1);
            for s in &steps[plan + 1..] {
                let cond = s["if"].as_str().unwrap_or_default();
                assert!(cond.starts_with("steps.plan.outputs.work == 'true'"), "{s}");
            }
        }
    }
}

/// Runs the docs-only step's script in a real git repo whose PR changes `changed` (`a->b`: a move).
fn docs_only(changed: &[&str]) -> String {
    docs_only_in(&["rust"], changed)
}

fn docs_only_in(stack: &[&str], changed: &[&str]) -> String {
    let files = variant(stack, "");
    let j = yaml_json(file(&files, BOSUN_YML)).unwrap();
    let steps = steps(&j, "");
    let script = step(&steps, "id", "diff")["run"].as_str().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let sh = |cmd: &str, base: &str, head: &str| {
        let o = Command::new("sh")
            .current_dir(dir.path())
            .env("GITHUB_OUTPUT", dir.path().join("out"))
            .env("BASE", base)
            .env("HEAD", head)
            .args(["-c", cmd])
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    let git = |cmd: &str| sh(cmd, "", "");
    git(
        "git init -q -b main && git config user.email a@b.c && git config user.name n \
        && mkdir docs src && echo a > docs/old.md && echo a > src/old.rs \
        && git add . && git commit -qm base",
    );
    let base = git("git rev-parse HEAD").trim().to_string();
    for c in changed {
        if let Some((from, to)) = c.split_once("->") {
            git(&format!("mkdir -p $(dirname {to}) && git mv {from} {to}"));
        } else {
            git(&format!("mkdir -p $(dirname {c}) && echo x >> {c}"));
        }
    }
    git("git add -A && git commit -qm pr --allow-empty");
    let head = git("git rev-parse HEAD").trim().to_string();
    sh(script, &base, &head);
    fs::read_to_string(dir.path().join("out"))
        .unwrap()
        .trim()
        .to_string()
}

#[test]
fn docs_only_detection_fails_closed() {
    if yaml_json("a: 1").is_none() {
        return;
    }
    let all = ["docs/new/a.md", "plans/p.md", "CHANGELOG.md"];
    assert_eq!(docs_only(&all), "docs_only=true");
    // A crate may run README.md examples as doctests (`include_str!`), so it runs the gates.
    assert_eq!(docs_only(&["README.md"]), "docs_only=false");
    assert_eq!(docs_only(&["docs/a.md", "src/x.rs"]), "docs_only=false");
    assert_eq!(docs_only(&["src/a.md"]), "docs_only=false");
    // Only markdown is docs: code or data under docs/ still runs the gates.
    assert_eq!(docs_only(&["docs/x.rs"]), "docs_only=false");
    assert_eq!(docs_only(&["docs/a.md", "docs/b.json"]), "docs_only=false");
    // ts-pnpm: prettier checks markdown, so there is no fast path at all.
    for stack in [&["ts-pnpm"][..], &["rust", "ts-pnpm"]] {
        assert_eq!(docs_only_in(stack, &["docs/a.md"]), "docs_only=false");
    }
    // A move out of src/ into docs/ still touches src/.
    assert_eq!(docs_only(&["src/old.rs->docs/old.rs"]), "docs_only=false");
    assert_eq!(docs_only(&[]), "docs_only=false", "an empty diff");
}

#[test]
fn a_body_edit_waits_and_never_cancels() {
    let files = variant(&["rust"], "1");
    let wf = file(&files, BOSUN_YML);
    let Some(j) = yaml_json(wf) else { return };
    let g = j["concurrency"]["group"].as_str().unwrap();
    assert!(
        !g.contains("run_id") && g.contains("github.event_name"),
        "{g}"
    );
    let c = j["concurrency"]["cancel-in-progress"].as_str().unwrap();
    assert!(
        c.contains("!(github.event.action == 'edited' && !github.event.changes.base)"),
        "{c}"
    );
    assert_eq!(j["permissions"]["checks"], "read");
}

/// Runs the edit step with a fake `gh` that applies its `--jq` filter (with jq) to a check-runs
/// answer: `runs` are `(id, app, status, conclusion, run id)`; returns the step's `mode` output.
fn edit_runs(runs: &[(u64, &str, &str, &str, u64)]) -> String {
    let files = variant(&["rust"], "");
    let j = yaml_json(file(&files, BOSUN_YML)).unwrap();
    let steps = steps(&j, "");
    let s = step(&steps, "id", "edit");
    assert_eq!(
        s["if"],
        "github.event.action == 'edited' && !github.event.changes.base"
    );
    // The body never decides: test-integrity re-reads it in this run.
    assert!(!s["env"].to_string().contains("body"), "{s}");
    let dir = tempfile::tempdir().unwrap();
    let runs: Vec<serde_json::Value> = runs
        .iter()
        .map(|(id, app, status, conclusion, run)| {
            serde_json::json!({"id": id, "app": {"slug": app}, "status": status,
                "conclusion": if conclusion.is_empty() { serde_json::Value::Null } else { (*conclusion).into() },
                "details_url": format!("https://github.com/o/r/actions/runs/{run}/job/{id}")})
        })
        .collect();
    let answer = dir.path().join("answer.json");
    fs::write(
        &answer,
        serde_json::json!({ "check_runs": runs }).to_string(),
    )
    .unwrap();
    let gh = dir.path().join("gh");
    // `gh api URL --jq EXPR`: the URL must name the head and the check.
    fs::write(
        &gh,
        format!(
            "#!/bin/sh\ncase \"$2\" in repos/o/r/commits/abc/check-runs?check_name=ci-ok\\&*) ;; *) exit 9;; esac\n\
             [ \"$3\" = --jq ] || exit 9\njq -r \"$4\" {}\n",
            answer.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        dir.path().display(),
        std::env::var("PATH").unwrap()
    );
    let out = dir.path().join("out");
    let ok = Command::new("bash")
        .args(["-eo", "pipefail", "-c", s["run"].as_str().unwrap()])
        .env("PATH", path)
        .env("GITHUB_OUTPUT", &out)
        .env("GITHUB_RUN_ID", "7")
        .env("GITHUB_REPOSITORY", "o/r")
        .env("HEAD", "abc")
        .env("CHECK", "ci-ok")
        .status()
        .unwrap()
        .success();
    assert!(ok, "the edit step failed");
    fs::read_to_string(out).unwrap().trim().to_string()
}

#[test]
fn a_body_edit_keeps_the_file_gates_of_a_passed_head_and_reruns_test_integrity() {
    if yaml_json("a: 1").is_none() {
        return;
    }
    let one = |c: &str| edit_runs(&[(5, "github-actions", "completed", c, 3)]);
    assert_eq!(one("success"), "mode=integrity");
    // A failure (maybe test-integrity on an older body), a cancel, a skip or nothing: every gate.
    for prev in ["failure", "cancelled", "skipped", "neutral"] {
        assert_eq!(one(prev), "mode=run", "{prev:?}");
    }
    assert_eq!(edit_runs(&[]), "mode=run", "no earlier run");
    // The newest finished run of GitHub Actions counts; this run's own and other apps' do not.
    let ok = (4, "github-actions", "completed", "success", 3);
    let failed = (6, "github-actions", "completed", "failure", 4);
    assert_eq!(edit_runs(&[ok, failed]), "mode=run");
    assert_eq!(edit_runs(&[failed, ok]), "mode=run", "by id, not order");
    let mine = (9, "github-actions", "completed", "success", 7);
    assert_eq!(edit_runs(&[failed, mine]), "mode=run");
    let other_app = (9, "forger", "completed", "success", 8);
    assert_eq!(edit_runs(&[failed, other_app]), "mode=run");
    let running = (9, "github-actions", "in_progress", "", 8);
    assert_eq!(edit_runs(&[ok, running]), "mode=integrity");
}

#[test]
fn main_open_skips_label_events_but_the_mainfix_label_in_a_group_of_their_own() {
    let ours = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows/main-open.yml");
    let files = variant(&["rust"], "");
    for wf in [
        file(&files, ".github/workflows/main-open.yml").to_string(),
        fs::read_to_string(ours).unwrap(),
    ] {
        let Some(j) = yaml_json(&wf) else { return };
        let cond = j["jobs"]["judge"]["if"].as_str().unwrap();
        assert!(
            cond.contains("github.event.action != 'labeled' && github.event.action != 'unlabeled'"),
            "{cond}"
        );
        assert!(
            cond.contains("|| github.event.label.name == 'bosun:kind/mainfix'"),
            "{cond}"
        );
        let g = j["concurrency"]["group"].as_str().unwrap();
        assert!(g.contains("github.event.label.name != 'bosun:kind/mainfix' && format('-skip-{0}', github.run_id)"), "{g}");
    }
}

#[test]
fn risk_rules_put_every_agent_rule_file_under_r_gov() {
    use bosun::core::risk::{ChangedFile, Rules, evaluate};
    let files = variant(&["rust"], "");
    let rules = Rules::parse(file(&files, ".bosun/risk.toml")).unwrap();
    let hit = |p: &str| {
        let f = ChangedFile {
            path: p.into(),
            ..Default::default()
        };
        evaluate(&rules, &[f], &[], &[])
            .iter()
            .any(|m| m.rule == "R-gov")
    };
    for p in [
        "AGENTS.md",
        "CLAUDE.md",
        "REVIEW.md",
        "docs/agents/project.md",
        "crates/foo/AGENTS.md",
        "crates/foo/CLAUDE.md",
    ] {
        assert!(hit(p), "{p} is not R-gov");
    }
    assert!(!hit("crates/foo/src/lib.rs") && !hit("docs/guide.md"));
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

/// Runs the rust toolchain step in `cwd` under the workspace `root`, with a fake
/// `rustup` on PATH; returns the rustup calls it made.
fn toolchain_calls(root: &std::path::Path, cwd: &std::path::Path) -> String {
    let files = variant(&["rust"], "");
    let j = yaml_json(file(&files, BOSUN_YML)).unwrap();
    let steps = steps(&j, "");
    let run = step(&steps, "name", "rust toolchain")["run"]
        .as_str()
        .unwrap();
    let bin = tempfile::tempdir().unwrap();
    let log = bin.path().join("log");
    let fake = bin.path().join("rustup");
    fs::write(
        &fake,
        format!("#!/bin/sh\necho \"$*\" >> {}\n", log.display()),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!(
        "{}:{}",
        bin.path().display(),
        std::env::var("PATH").unwrap()
    );
    let ok = Command::new("bash")
        .args(["-eo", "pipefail", "-c", run])
        .current_dir(cwd)
        .env("PATH", path)
        .env("GITHUB_WORKSPACE", root)
        .status()
        .unwrap()
        .success();
    assert!(ok, "the toolchain step failed");
    fs::read_to_string(log).unwrap_or_default()
}

#[test]
fn the_toolchain_step_installs_the_file_or_the_old_pin() {
    if yaml_json("a: 1").is_none() {
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let sub = root.path().join("crates/core");
    fs::create_dir_all(&sub).unwrap();
    // No toolchain file anywhere: the old pin, never the runner's default (stable).
    let calls = toolchain_calls(root.path(), &sub);
    assert!(calls.starts_with("toolchain install 1.97 "), "{calls}");
    assert!(calls.contains("default 1.97"), "{calls}");
    // The seeded root file, found from a rust_path below it: rustup installs what it names.
    fs::write(
        root.path().join("rust-toolchain.toml"),
        "[toolchain]\nchannel = \"1.99.1\"\n",
    )
    .unwrap();
    for cwd in [root.path(), sub.as_path()] {
        let calls = toolchain_calls(root.path(), cwd);
        assert!(calls.starts_with("toolchain install\n"), "{calls}");
        assert!(!calls.contains("1.97"), "{calls}");
    }
}

/// Runs the plan step: `Some(outputs)` when it passes, `None` when it fails the job.
fn plan(own_ci: &str, draft: &str, docs_only: &str, mode: &str) -> Option<String> {
    let files = variant(&["rust"], own_ci);
    let j = yaml_json(file(&files, BOSUN_YML)).unwrap();
    let steps = steps(&j, own_ci);
    let run = step(&steps, "id", "plan")["run"].as_str().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    let ok = Command::new("bash")
        .args(["-eo", "pipefail", "-c", run])
        .env("GITHUB_OUTPUT", &out)
        .env("DRAFT", draft)
        .env("DOCS_ONLY", docs_only)
        .env("MODE", mode)
        .output()
        .unwrap()
        .status
        .success();
    ok.then(|| fs::read_to_string(out).unwrap().trim().to_string())
}

#[test]
fn the_plan_skips_gates_only_on_docs_and_drafts_fail() {
    let files = variant(&["rust"], "");
    let wf = file(&files, BOSUN_YML);
    assert!(
        !wf.contains("if: github.event_name != 'pull_request' || !github.event.pull_request.draft")
    );
    if yaml_json("a: 1").is_none() {
        return;
    }
    for own_ci in ["", "1"] {
        let full = Some("work=true\nsetup=true".to_string());
        let integrity = Some("work=true\nsetup=false\nonly=test-integrity".to_string());
        let idle = Some("work=false".to_string());
        assert_eq!(plan(own_ci, "false", "false", "run"), full);
        assert_eq!(plan(own_ci, "false", "", "run"), full, "not a pull request");
        assert_eq!(plan(own_ci, "", "", "run"), full, "no draft flag");
        assert_eq!(plan(own_ci, "false", "true", "run"), idle);
        assert_eq!(plan(own_ci, "false", "false", "integrity"), integrity);
        assert_eq!(
            plan(own_ci, "false", "true", "integrity"),
            idle,
            "docs only"
        );
        // A draft fails at once, never skipped: a skipped required check counts as passed.
        for mode in ["run", "integrity"] {
            assert_eq!(plan(own_ci, "true", "false", mode), None);
            assert_eq!(plan(own_ci, "true", "true", mode), None);
        }
        // Only the stack setup waits for `setup`; the gates step takes `only`.
        let j = yaml_json(file(&variant(&["rust", "ts-pnpm"], own_ci), BOSUN_YML)).unwrap();
        let steps = steps(&j, own_ci);
        let setup: Vec<_> = steps
            .iter()
            .filter(|s| {
                s["if"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("steps.plan.outputs.setup == 'true'")
            })
            .collect();
        assert_eq!(
            setup.len(),
            7,
            "toolchain, rust-cache, install-action, 4 ts-pnpm steps"
        );
        let gates = step(&steps, "name", "bosun gates");
        assert_eq!(gates["env"]["ONLY"], "${{ steps.plan.outputs.only }}");
        assert!(
            gates["run"]
                .as_str()
                .unwrap()
                .contains("GATES=\"${ONLY:-$GATES}\"")
        );
    }
}
