//! `bosun doctor`: report each step of the human checklist (spec §11) as ok or missing.

use std::path::Path;

use serde_json::{Value, json};

use crate::app::{App, HALT_LABEL, halt_reason};
use crate::github::{Github, IssueState};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;

const REQUIRED_CHECKS: [&str; 4] = ["ci-ok", "risk-check", "review-verdict", "main-open"];
const HOIST_FIX: &str = "run `hoist plan --area github && hoist apply --area github --yes` from a checkout of main with an admin token (docs/PROGRESS.md Blocked)";

fn item(step: u8, name: &str, problem: Option<String>, ok_detail: String, fix: &str) -> Value {
    let (status, detail) = match problem {
        None => ("ok", ok_detail),
        Some(p) => ("missing", p),
    };
    json!({"step": step, "name": name, "status": status, "detail": detail, "fix": fix})
}

/// The Routine prompts the owner creates by hand (step 11).
fn routines() -> Value {
    json!([
        {"name": "janitor", "schedule": "every 15 minutes", "model": "haiku",
         "prompt": "Run `bosun reap`. List stale sessions with `bosun status` and report any session whose lease expired. Check the budget in `bosun status` (budget_pct); at 100 % stop the live sessions, at 80 % post a warning. Then run `bosun notify`."},
        {"name": "curator", "schedule": "daily", "model": "sonnet",
         "prompt": "Triage the open lesson issues as in docs/spec.md section 6: a blocking lesson sets process-red; a mechanizable one opens a kind/process-fix task; advice only goes into docs/lessons.toml with `bosun lesson add`. Retire lessons with no hit in 30 days. Close a lesson issue when its mechanism has merged."},
        {"name": "notify", "schedule": "every 15 minutes", "model": "haiku",
         "prompt": "Run `bosun notify` and post what it prints to the Slack channel from .bosun/bosun.toml."},
    ])
}

fn read(root: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(root.join(rel)).ok()
}

fn has_owner(v: &toml::Value) -> bool {
    match v {
        toml::Value::Table(t) => t.iter().any(|(k, x)| {
            let named = k.starts_with("owner");
            (named
                && match x {
                    toml::Value::Array(a) => !a.is_empty(),
                    toml::Value::Table(t) => !t.is_empty(),
                    toml::Value::String(s) => !s.trim().is_empty(),
                    _ => false,
                })
                || has_owner(x)
        }),
        toml::Value::Array(a) => a.iter().any(has_owner),
        _ => false,
    }
}

fn app_user(login: &str) -> bool {
    login.to_lowercase().starts_with("claude")
}

fn claude_app_seen(gh: &dyn Github) -> Result<bool, BosunError> {
    let mut issues = gh.list_issues(&[], IssueState::All)?;
    issues.truncate(50);
    if issues.iter().any(|i| app_user(&i.user)) {
        return Ok(true);
    }
    let mut pulls = gh.list_pulls(None, IssueState::All)?;
    pulls.truncate(50);
    if pulls.iter().any(|p| app_user(&p.user)) {
        return Ok(true);
    }
    let numbers = issues
        .iter()
        .map(|i| i.number)
        .chain(pulls.iter().map(|p| p.number))
        .take(10);
    for n in numbers {
        if gh.list_comments(n)?.iter().any(|c| app_user(&c.user)) {
            return Ok(true);
        }
    }
    Ok(false)
}

const HOIST_INSTALL_FIX: &str =
    "install hoist (`npm install -g @keelstackhq/hoist`) or set BOSUN_HOIST";
const HOIST_RERUN_FIX: &str = "run `hoist check` by hand and fix what it reports";
const HOIST_TIMEOUT_SECS: u64 = 120;

/// The step and `hoist apply` area for a `hoist check` item (D17): 3 = repo settings, rulesets,
/// required checks; 5 = environments, secrets, variables; 10 = first publish of npm/crates.
fn hoist_step(area: &str, kind: &str) -> (usize, String) {
    match (area, kind) {
        ("npm" | "crates", _) => (10, area.into()),
        ("release", _) => (5, "release".into()),
        (_, "environment" | "secret" | "variable") => (5, "github".into()),
        _ => (3, "github".into()),
    }
}

fn describe(it: &Value) -> String {
    let s = |k: &str| it[k].as_str().unwrap_or("");
    format!(
        "{}/{}/{}: {} ({})",
        s("area"),
        s("kind"),
        s("key"),
        s("action"),
        s("reason")
    )
}

/// Add hoist's finding to a step, keeping the step's own problem and fix.
fn append_problem(it: &mut Value, problem: &str, fix: &str) {
    let (detail, fix) = if it["status"] == "missing" {
        (
            format!("{}; {problem}", it["detail"].as_str().unwrap_or("")),
            format!("{}; then {fix}", it["fix"].as_str().unwrap_or("")),
        )
    } else {
        (problem.to_string(), fix.to_string())
    };
    it["status"] = json!("missing");
    it["detail"] = json!(detail);
    it["fix"] = json!(fix);
}

fn hoist_failed(items: &mut [Value], detail: &str, fix: &str, exit_code: Value) -> Value {
    for step in [3, 5] {
        append_problem(&mut items[step - 1], detail, fix);
    }
    json!({"exit_code": exit_code, "drift": [], "unknown": [], "error": detail})
}

type HoistRun = Result<(Option<i32>, Vec<u8>), Option<std::io::Error>>;

/// Run hoist with stdout captured and a deadline; `Err(None)` means it timed out.
fn run_hoist(bin: &str, root: &Path, secs: u64) -> HoistRun {
    use std::io::Read;
    // The token reaches hoist through the inherited environment, never argv.
    let mut child = std::process::Command::new(bin)
        .args(["check", "--json", "-C"])
        .arg(root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .map_err(Some)?;
    let mut pipe = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(p) = pipe.as_mut() {
            let _ = p.read_to_end(&mut buf);
        }
        buf
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let status = loop {
        match child.try_wait().map_err(Some)? {
            Some(s) => break s,
            None if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(None);
            }
            None => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    };
    Ok((status.code(), reader.join().unwrap_or_default()))
}

#[derive(Default)]
struct StepFindings {
    drift: Vec<String>,
    unknown: Vec<String>,
    areas: Vec<String>,
}

/// Run `hoist check --json` and fold its items into steps 3, 5 and 10. Returns `data.hoist`.
fn apply_hoist(items: &mut [Value], root: &Path) -> Value {
    let bin = std::env::var("BOSUN_HOIST")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "hoist".into());
    let secs = std::env::var("BOSUN_HOIST_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(HOIST_TIMEOUT_SECS);
    let (code, stdout) = match run_hoist(&bin, root, secs) {
        Ok(r) => r,
        Err(None) => {
            let detail = format!("hoist check timed out after {secs}s");
            return hoist_failed(items, &detail, HOIST_RERUN_FIX, Value::Null);
        }
        Err(Some(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            let d = "hoist is not installed";
            return hoist_failed(items, d, HOIST_INSTALL_FIX, Value::Null);
        }
        Err(Some(e)) => {
            let detail = format!("cannot run {bin}: {e}");
            return hoist_failed(items, &detail, HOIST_INSTALL_FIX, Value::Null);
        }
    };
    let exit_json = code.map_or(Value::Null, |c| json!(c));
    let report = serde_json::from_slice::<Value>(&stdout)
        .ok()
        .filter(|v| v["items"].is_array());
    let Some(report) = report else {
        let detail = format!("hoist check failed (exit {exit_json}): no JSON report on stdout");
        return hoist_failed(items, &detail, HOIST_RERUN_FIX, exit_json);
    };
    if !code.is_some_and(|c| c <= 1) {
        let detail = format!("hoist check exited with {exit_json} (expected 0 or 1)");
        return hoist_failed(items, &detail, HOIST_RERUN_FIX, exit_json);
    }
    let (mut drift, mut unknown) = (Vec::new(), Vec::new());
    let mut per_step: std::collections::BTreeMap<usize, StepFindings> = [3, 5, 10]
        .into_iter()
        .map(|s| (s, StepFindings::default()))
        .collect();
    for it in report["items"].as_array().into_iter().flatten() {
        let s = |k: &str| it[k].as_str().unwrap_or("").to_string();
        let (step, area) = hoist_step(&s("area"), &s("kind"));
        let entry = per_step.entry(step).or_default();
        let action = s("action");
        let verify_bad = it["verify"]["ok"] == false;
        if action == "unknown" {
            unknown.push(describe(it));
            entry.unknown.push(format!("{}/{}", s("kind"), s("key")));
        } else if action != "ok" || verify_bad {
            let line = if action == "ok" {
                format!(
                    "{}/{}/{}: verify failed ({})",
                    s("area"),
                    s("kind"),
                    s("key"),
                    it["verify"]["detail"].as_str().unwrap_or("")
                )
            } else {
                describe(it)
            };
            if !entry.areas.contains(&area) {
                entry.areas.push(area);
            }
            entry.drift.push(line.clone());
            drift.push(line);
        }
    }
    for (step, f) in per_step {
        let it = &mut items[step - 1];
        if !f.drift.is_empty() {
            let fix = f
                .areas
                .iter()
                .map(|a| format!("`hoist apply --area {a} --yes`"))
                .collect::<Vec<_>>()
                .join(" and ");
            let problem = format!("hoist check drift: {}", f.drift.join("; "));
            append_problem(it, &problem, &format!("run {fix}"));
        } else if it["status"] == "ok" {
            let note = if f.unknown.is_empty() {
                "hoist check: no drift".to_string()
            } else {
                format!(
                    "hoist could not read {} item(s): {}",
                    f.unknown.len(),
                    f.unknown.join(", ")
                )
            };
            it["detail"] = json!(format!("{}; {note}", it["detail"].as_str().unwrap_or("")));
        }
    }
    json!({"exit_code": exit_json, "drift": drift, "unknown": unknown})
}

pub fn run(dir: Option<&Path>, remote: bool) -> CmdResult {
    let app = App::load(dir)?;
    let (gh, ctx) = (app.gh.as_ref(), &app.ctx);
    let root = ctx.dir.as_path();
    let mut items = Vec::new();

    let repo = gh.repo_info();
    items.push(item(
        1,
        "repo readable",
        match &repo {
            Ok(Some(_)) => None,
            Ok(None) => Some(format!("{} is not readable with this token", ctx.repo)),
            Err(e) => Some(e.message.clone()),
        },
        format!(
            "{} (default branch {})",
            ctx.repo,
            repo.as_ref()
                .ok()
                .and_then(|r| r.as_ref())
                .map_or("", |r| &r.default_branch)
        ),
        "create the repo, and give the GitHub token access to it",
    ));

    let seen = claude_app_seen(gh);
    items.push(item(
        2,
        "Claude GitHub App",
        match &seen {
            Ok(true) => None,
            Ok(false) => Some(
                "no PR, issue or comment by a `claude` login in the last 50 issues and PRs".into(),
            ),
            Err(e) => Some(e.message.clone()),
        },
        "a `claude` login has written here".into(),
        "install the app from https://github.com/apps/claude and grant it this repo",
    ));

    let hoist = read(root, "hoist.toml").and_then(|t| t.parse::<toml::Table>().ok());
    let listed: Vec<String> = hoist
        .as_ref()
        .and_then(|h| h.get("github")?.get("required_checks")?.as_array().cloned())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let absent: Vec<_> = REQUIRED_CHECKS
        .iter()
        .filter(|c| !listed.iter().any(|l| l == *c))
        .collect();
    let ci_ok_job = read(root, ".github/workflows/ci.yml")
        .is_some_and(|t| t.lines().any(|l| l.trim_end() == "  ci-ok:"));
    items.push(item(
        3,
        "required checks",
        if hoist.is_none() {
            Some("hoist.toml is missing or not valid TOML".into())
        } else if !absent.is_empty() {
            Some(format!(
                "hoist.toml [github] required_checks lacks {absent:?}"
            ))
        } else if !ci_ok_job {
            Some(".github/workflows/ci.yml has no `ci-ok` job".into())
        } else {
            None
        },
        "hoist.toml lists the four checks and ci.yml has the ci-ok job".into(),
        HOIST_FIX,
    ));

    let owners = read(root, "governance/owners.toml")
        .and_then(|t| t.parse::<toml::Table>().ok())
        .map(toml::Value::Table);
    items.push(item(
        4,
        "owners",
        (!owners.as_ref().is_some_and(has_owner))
            .then(|| "governance/owners.toml has no owner".into()),
        "governance/owners.toml names an owner".into(),
        "add yourself to governance/owners.toml",
    ));

    // The kill switch is an open `bosun:halt` issue by an owner; BOSUN_HALT is optional.
    let label = gh.label_exists(HALT_LABEL);
    let halt = halt_reason(gh);
    items.push(item(
        5,
        "kill switch",
        match (&label, &halt) {
            (_, Err(e)) => Some(format!("cannot read the kill switch: {}", e.message)),
            (Err(e), _) => Some(format!("cannot read the label {HALT_LABEL}: {}", e.message)),
            (Ok(false), _) => Some(format!("the label {HALT_LABEL} does not exist")),
            (Ok(true), Ok(_)) => None,
        },
        match &halt {
            Ok(Some((reason, _))) => format!("halted: {reason}"),
            _ => format!("not halted; an owner's open {HALT_LABEL} issue stops all work"),
        },
        "gh label create bosun:halt --color B60205 --repo <owner>/<repo>",
    ));

    let version = read(root, ".bosun/version").map(|v| v.trim().to_string());
    let script = root.join("docs/setup-script.sh").is_file();
    items.push(item(
        6,
        "setup script and version",
        if !script {
            Some("docs/setup-script.sh is missing".into())
        } else if version.is_none() {
            Some(".bosun/version is missing".into())
        } else {
            None
        },
        format!("docs/setup-script.sh exists; .bosun/version is {}", version.clone().unwrap_or_default()),
        "paste docs/setup-script.sh into the cloud environment's setup script (claude.ai/code) and keep .bosun/version",
    ));

    let channel = ctx
        .config
        .slack
        .as_ref()
        .map(|s| s.channel.trim().to_string())
        .unwrap_or_default();
    items.push(item(
        7,
        "Slack channel",
        channel
            .is_empty()
            .then(|| "[slack] channel is not set in .bosun/bosun.toml".into()),
        format!("[slack] channel = {channel}"),
        "create the channel, connect Claude Tag, and set [slack] channel in .bosun/bosun.toml",
    ));

    let reg = registry::load(gh);
    items.push(item(
        8,
        "registry issue",
        match &reg {
            Ok(Some(_)) => None,
            Ok(None) => Some("no open issue labelled bosun:registry".into()),
            Err(e) => Some(e.message.clone()),
        },
        format!(
            "registry issue #{}",
            reg.as_ref()
                .ok()
                .and_then(|r| r.as_ref())
                .map_or(0, |r| r.0)
        ),
        "start the seed session; `bosun join` creates the registry",
    ));

    items.push(item(
        9,
        "auto-merge",
        match &repo {
            Ok(Some(r)) if r.allow_auto_merge => None,
            Ok(Some(_)) => Some("allow_auto_merge is off".into()),
            _ => Some("the repo is not readable".into()),
        },
        "allow_auto_merge is on".into(),
        HOIST_FIX,
    ));

    let release = gh.latest_release();
    items.push(item(
        10,
        "release",
        match &release {
            Ok(Some(_)) => None,
            Ok(None) => Some("no release yet".into()),
            Err(e) => Some(e.message.clone()),
        },
        format!(
            "latest release {}",
            release
                .as_ref()
                .ok()
                .and_then(|r| r.as_ref())
                .map_or("", |r| &r.tag_name)
        ),
        "run `hoist release bootstrap` (docs/PROGRESS.md Blocked)",
    ));

    let routines_ok = std::env::var("BOSUN_ROUTINES_OK").is_ok_and(|v| v.trim() == "1");
    items.push(item(
        11,
        "Routines",
        (!routines_ok).then(|| "bosun cannot see Routines; create them from the prompts in data.routines".into()),
        "BOSUN_ROUTINES_OK=1: the owner confirmed the three Routines".into(),
        "create the janitor, curator and notify Routines from data.routines, then set BOSUN_ROUTINES_OK=1",
    ));

    let has_plan = std::fs::read_dir(root.join("plans")).is_ok_and(|d| {
        d.filter_map(Result::ok)
            .any(|e| e.path().extension().is_some_and(|x| x == "md"))
    });
    items.push(item(
        12,
        "plan",
        (!has_plan).then(|| "no plans/*.md".into()),
        "plans/ holds a plan".into(),
        "write plans/<name>.md in the plan format (docs/plan-format.md) and approve the plan PR",
    ));

    let hoist_data = remote.then(|| apply_hoist(&mut items, root));
    let missing = items.iter().filter(|i| i["status"] == "missing").count();
    let mut data = json!({"items": items, "missing": missing, "routines": routines()});
    if let Some(h) = hoist_data {
        data["hoist"] = h;
    }
    if missing > 0 {
        return Err(BosunError::needs_human(
            format!("{missing} checklist steps missing"),
            "do the missing steps in data.items; see docs/PROGRESS.md Blocked",
        )
        .with_data(data));
    }
    Ok(Outcome::data(data))
}
