//! `bosun doctor`: report each step of the human checklist (spec §11) as ok or missing.

use std::path::Path;

use serde_json::{Value, json};

use crate::app::{App, HALT_LABEL, halt_reason};
use crate::core::checks;
use crate::github::{Github, IssueState};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;

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
         "prompt": "Run `bosun reap` (it ends expired leases and members and moves merged tasks to merged) and report its data.reaped, data.left and data.merged. Run `bosun budget check`: make only the interrupt_session calls in its `next`, with the claude-code-remote tools (one per live session at 100 % of the daily budget), and report data.level. Then run `bosun notify` if this bosun has it."},
        {"name": "curator", "schedule": "daily", "model": "sonnet",
         "prompt": "List the open issues labelled `bosun:lesson` that have no `bosun:lesson/*` label. For each one run `bosun lesson triage <n> --as blocking|mechanizable|advice` as in docs/spec.md section 6: blocking when the process must stop until it is fixed; mechanizable when a gate, a hook or a check can prevent it (add `--touch GLOB` for the paths it changes and `--area A`); advice only as the last choice (add `--rule TEXT` and `--scope GLOB`). Prefer mechanizable over advice. Add other rules with `bosun lesson add`; at the cap (exit 6) mechanize, combine or retire a lesson first, and retire lessons with no hit in 30 days. Close a lesson issue when its mechanism has landed."},
        {"name": "notify", "schedule": "every 15 minutes", "model": "haiku",
         "prompt": "Run `bosun notify` and post what it prints to the Slack channel from .bosun/bosun.toml."},
    ])
}

/// Check contexts of every `.github/workflows/*.yml` and `*.yaml` (see `checks::workflow_checks`).
fn workflow_jobs(root: &Path) -> Vec<String> {
    let Ok(dir) = std::fs::read_dir(root.join(".github/workflows")) else {
        return Vec::new();
    };
    dir.flatten()
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x == "yml" || x == "yaml")
        })
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .flat_map(|t| checks::workflow_checks(&t))
        .collect()
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
    json!({"exit_code": exit_code, "drift": [], "unknown": [], "notes": [], "error": detail})
}

type HoistRun = Result<(Option<i32>, Vec<u8>), Option<std::io::Error>>;

/// Kill hoist and everything it started (its pgid is its pid; see `process_group(0)`).
fn kill_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", child.id())])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    let _ = child.kill();
}

/// Run hoist with stdout captured and a deadline; `Err(None)` means it timed out.
fn run_hoist(bin: &str, root: &Path, secs: u64) -> HoistRun {
    use std::io::Read;
    // The token reaches hoist through the inherited environment, never argv.
    let mut cmd = std::process::Command::new(bin);
    cmd.args(["check", "--json", "-C"])
        .arg(root)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit());
    // Own process group, so a timeout can kill the node shim's native child too.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = cmd.spawn().map_err(Some)?;
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
                kill_group(&mut child);
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
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
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
    let (mut drift, mut unknown, mut notes) = (Vec::new(), Vec::new(), Vec::new());
    let mut per_step: std::collections::BTreeMap<usize, StepFindings> = [3, 5, 10]
        .into_iter()
        .map(|s| (s, StepFindings::default()))
        .collect();
    for it in report["items"].as_array().into_iter().flatten() {
        let s = |k: &str| it[k].as_str().unwrap_or("").to_string();
        let (step, area) = hoist_step(&s("area"), &s("kind"));
        let entry = per_step.entry(step).or_default();
        let action = s("action");
        // A failed verify is a note, never drift: hoist's job-name check cannot see commit
        // statuses such as risk-check, so it would fail forever on a correct repo.
        if action == "ok" && it["verify"]["ok"] == false {
            notes.push(format!(
                "{}/{}/{}: {}",
                s("area"),
                s("kind"),
                s("key"),
                it["verify"]["detail"].as_str().unwrap_or("verify failed")
            ));
        }
        if action == "unknown" {
            unknown.push(describe(it));
            entry.unknown.push(format!("{}/{}", s("kind"), s("key")));
        } else if action != "ok" {
            let line = describe(it);
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
    json!({"exit_code": exit_json, "drift": drift, "unknown": unknown, "notes": notes})
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
    let jobs = workflow_jobs(root);
    let has_job = |j: &str| jobs.iter().any(|x| x == j);
    let mut absent: Vec<&str> = checks::DEFAULT_REQUIRED.to_vec();
    if has_job("bosun-ok") {
        absent.push("bosun-ok");
    }
    absent.retain(|c| !listed.iter().any(|l| l == c));
    let step3_fix = if absent.is_empty() {
        HOIST_FIX.to_string()
    } else {
        format!("add {absent:?} to hoist.toml [github] required_checks, then {HOIST_FIX}")
    };
    items.push(item(
        3,
        "required checks",
        if hoist.is_none() {
            Some("hoist.toml is missing or not valid TOML".into())
        } else if !absent.is_empty() {
            Some(format!(
                "hoist.toml [github] required_checks lacks {absent:?}"
            ))
        } else if !has_job("ci-ok") {
            Some("no .github/workflows/*.yml has a `ci-ok` job".into())
        } else {
            None
        },
        "hoist.toml lists the required checks and a workflow has the ci-ok job".into(),
        &step3_fix,
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
    let mut slack_item = item(
        7,
        "Slack channel",
        channel
            .is_empty()
            .then(|| "[slack] channel is not set in .bosun/bosun.toml".into()),
        format!("[slack] channel = {channel}"),
        "create the channel, connect Claude Tag, and set [slack] channel in .bosun/bosun.toml",
    );
    // Without an owner Slack ID no answer counts and no thread may spawn: warn, not fail.
    let parsed = read(root, "governance/owners.toml").map(|t| crate::core::risk::parse_owners(&t));
    let has_slack_id = matches!(&parsed, Some(Ok(o)) if o.iter().any(|o| o.slack.is_some()));
    if !channel.is_empty() && !has_slack_id {
        slack_item["status"] = json!("warn");
        slack_item["detail"] = json!(if matches!(parsed, Some(Err(_))) {
            "governance/owners.toml does not parse as [[owner]] tables"
        } else {
            "[slack] channel is set but no owner in governance/owners.toml has a slack ID, so no Slack answer counts"
        });
        slack_item["fix"] =
            json!("add `slack = \"U…\"` to your [[owner]] table in governance/owners.toml");
    }
    items.push(slack_item);

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
