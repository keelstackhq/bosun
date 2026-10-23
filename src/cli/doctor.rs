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

/// Top-level job names (two-space-indented `name:` lines) of every `.github/workflows/*.yml`.
fn workflow_jobs(root: &Path) -> Vec<String> {
    let Ok(dir) = std::fs::read_dir(root.join(".github/workflows")) else {
        return Vec::new();
    };
    dir.flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "yml"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .flat_map(|t| {
            t.lines()
                .filter_map(|l| {
                    let l = l.trim_end().strip_prefix("  ")?;
                    let name = l.strip_suffix(':')?;
                    (!name.starts_with(' ') && !name.contains(' ')).then(|| name.to_string())
                })
                .collect::<Vec<_>>()
        })
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

pub fn run(dir: Option<&Path>) -> CmdResult {
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
    let mut absent: Vec<&str> = REQUIRED_CHECKS.to_vec();
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

    let missing = items.iter().filter(|i| i["status"] == "missing").count();
    let data = json!({"items": items, "missing": missing, "routines": routines()});
    if missing > 0 {
        return Err(BosunError::needs_human(
            format!("{missing} checklist steps missing"),
            "do the missing steps in data.items; see docs/PROGRESS.md Blocked",
        )
        .with_data(data));
    }
    Ok(Outcome::data(data))
}
