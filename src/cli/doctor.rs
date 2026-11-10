//! `bosun doctor`: report each step of the human checklist (spec §11) as ok or missing.

use std::path::Path;

use serde_json::{Value, json};

use crate::app::{HALT_LABEL, halt_reason};
use crate::cli::pin::Check;
use crate::core::checks;
use crate::ctx::Ctx;
use crate::github::{EnvBranches, Github, IssueState, Live, LiveGithub};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;

const HOIST_FIX: &str = "run `hoist plan --area github && hoist apply --area github --yes` from a checkout of main with an admin token";

const NO_TOKEN_FIX: &str =
    "export GITHUB_TOKEN (repo and workflow scopes), then rerun `bosun doctor`";

/// A step that needs GitHub while no token is set: reported, not counted as missing.
fn no_token(step: u8, name: &str) -> Value {
    skipped(
        step,
        name,
        "needs GitHub and no GITHUB_TOKEN or GH_TOKEN is set",
        NO_TOKEN_FIX,
    )
}

fn item(step: u8, name: &str, problem: Option<String>, ok_detail: String, fix: &str) -> Value {
    let (status, detail) = match problem {
        None => ("ok", ok_detail),
        Some(p) => ("missing", p),
    };
    json!({"step": step, "name": name, "status": status, "detail": detail, "fix": fix})
}

/// A step the repo has not opted into (Slack, release): reported, never missing or warn (F11).
fn skipped(step: u8, name: &str, detail: &str, fix: &str) -> Value {
    json!({"step": step, "name": name, "status": "skipped", "detail": detail, "fix": fix})
}

fn join_note(detail: String, note: &Option<String>) -> String {
    match note {
        Some(n) => format!("{detail}; {n}"),
        None => detail,
    }
}

const JANITOR: &str = "Run `bosun reap` (it ends expired leases and members and moves merged tasks to merged) and report its data.reaped, data.left and data.merged. Run `bosun budget check`: make only the interrupt_session calls in its `next`, with the claude-code-remote tools (one per live session at 100 % of the daily budget), and report data.level.";

/// The Routine prompts the owner creates by hand (step 11). Without a `[slack]` table the
/// janitor skips notify and there is no notify Routine (F17).
pub(crate) fn routines(slack: bool) -> Value {
    let janitor = if slack {
        format!("{JANITOR} Then run `bosun notify` if this bosun has it.")
    } else {
        JANITOR.to_string()
    };
    let mut all = json!([
        {"name": "janitor", "schedule": "every hour", "model": "haiku", "prompt": janitor},
        {"name": "curator", "schedule": "daily", "model": "sonnet",
         "prompt": "List the open issues labelled `bosun:lesson` that have no `bosun:lesson/*` label. For each one run `bosun lesson triage <n> --as blocking|mechanizable|advice` as in docs/spec.md section 6: blocking when the process must stop until it is fixed; mechanizable when a gate, a hook or a check can prevent it (add `--touch GLOB` for the paths it changes and `--area A`); advice only as the last choice (add `--rule TEXT` and `--scope GLOB`). Prefer mechanizable over advice. Add other rules with `bosun lesson add \"<rule, at most 160 characters>\" --evidence #<issue>` (the lesson issue or PR that proved it); at the cap (exit 6) mechanize, combine or retire a lesson first, and retire lessons with no hit in 30 days. Close a lesson issue when its mechanism has landed."},
        {"name": "notify", "schedule": "every hour", "model": "haiku",
         "prompt": "Run `bosun notify --channel-info -` with the channel facts read by your Slack tool (conversations.info; guests are the members whose users.info has is_restricted or is_ultra_restricted) as JSON on stdin: {\"id\", \"is_ext_shared\", \"is_org_shared\", \"is_shared\", \"guests\"}. Post each `slack_post` in `next` with `args.text` verbatim (into args.thread_ts when it is set), then run `bosun notify --ack <key>` only after that post succeeded. Report data.errors when it is not empty. On exit 5, post nothing and report data.reason."},
    ]);
    if !slack && let Some(a) = all.as_array_mut() {
        a.retain(|r| r["name"] != "notify");
    }
    all
}

/// Whether the repo publishes releases: hoist.toml has `[release]` or `[publish]`, a workflow is
/// `release.yml`/`release.yaml` or names a `release*` environment, a package is publishable, or a
/// release exists. Else
/// release (step 10, hoist's release items) is optional and skipped (F11).
fn release_configured(root: &Path, hoist: Option<&toml::Table>, released: bool) -> bool {
    let wf = root.join(".github/workflows");
    released
        || hoist.is_some_and(|h| h.contains_key("release") || h.contains_key("publish"))
        || ["release.yml", "release.yaml"]
            .iter()
            .any(|f| wf.join(f).is_file())
        || workflow_environments(root)
            .0
            .iter()
            .any(|e| e.starts_with("release"))
        || publishable_package(root)
}

/// A target hoist would publish: a Cargo `[package]` (the root or a workspace member) without
/// `publish = false` or `publish = []`, or a `package.json` without `"private": true`.
fn publishable_package(root: &Path) -> bool {
    let npm = read(root, "package.json")
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .is_some_and(|p| p.is_object() && p["private"] != true);
    let Some(cargo) = read(root, "Cargo.toml").and_then(|t| t.parse::<toml::Table>().ok()) else {
        return npm;
    };
    let ws = cargo.get("workspace").and_then(|w| w.as_table());
    let inherited = ws
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("publish"));
    let publishes = |t: &toml::Table| {
        t.get("package")
            .and_then(|p| p.as_table())
            .is_some_and(|p| {
                let publish = match p.get("publish") {
                    Some(toml::Value::Table(t)) if t.get("workspace").is_some() => inherited,
                    other => other,
                };
                !matches!(publish, Some(toml::Value::Boolean(false)))
                    && !matches!(publish, Some(toml::Value::Array(a)) if a.is_empty())
            })
    };
    let members = ws
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
        .into_iter()
        .flatten()
        .filter_map(|m| m.as_str())
        .flat_map(|m| match m.strip_suffix("/*") {
            Some(dir) => std::fs::read_dir(root.join(dir))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|e| e.path().join("Cargo.toml"))
                .collect::<Vec<_>>(),
            None => vec![root.join(m).join("Cargo.toml")],
        });
    npm || publishes(&cargo)
        || members
            .filter_map(|p| std::fs::read_to_string(p).ok()?.parse::<toml::Table>().ok())
            .any(|t| publishes(&t))
}

/// An item hoist reports only when it infers a publish target: the release, npm and crates areas.
fn publish_evidence(area: &str) -> bool {
    matches!(area, "release" | "npm" | "crates")
}

/// A hoist item about releases: the release, npm and crates areas, a `release*` environment, or
/// a secret such as HOIST_RELEASE_TOKEN.
fn release_item(area: &str, kind: &str, key: &str) -> bool {
    matches!(area, "release" | "npm" | "crates")
        || (kind == "environment" && key.starts_with("release"))
        || (kind == "secret" && key.to_ascii_uppercase().contains("RELEASE"))
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

/// Environments of every workflow job (see `checks::workflow_environments`), sorted, and the
/// jobs whose environment doctor cannot read, as (job, workflow file).
fn workflow_environments(root: &Path) -> (Vec<String>, Vec<(String, String)>) {
    let (mut names, mut unread) = (Vec::new(), Vec::new());
    let Ok(dir) = std::fs::read_dir(root.join(".github/workflows")) else {
        return (names, unread);
    };
    for e in dir.flatten() {
        let file = e.file_name().to_string_lossy().into_owned();
        let Some(t) = (file.ends_with(".yml") || file.ends_with(".yaml"))
            .then(|| std::fs::read_to_string(e.path()).ok())
            .flatten()
        else {
            continue;
        };
        for r in checks::workflow_environments(&t) {
            match r {
                Ok(n) => names.push(n),
                Err(job) => unread.push((job, file.clone())),
            }
        }
    }
    names.sort();
    names.dedup();
    unread.sort();
    (names, unread)
}

const ENV_GRANT_FIX: &str = "give the token Environments: read (or Administration: read), then run `bosun doctor --remote` again";

/// One environment: ok only when its custom deployment branches are exactly `main`.
fn environment_check(gh: &dyn Github, repo: &str, name: &str) -> Value {
    let page = format!("https://github.com/{repo}/settings/environments");
    let steps = format!(
        "Settings → Environments → {name} → Deployment branches → Selected branches → add `main` and remove every other rule ({page})"
    );
    let warn = |detail: String, fix: String| json!({"name": name, "status": "warn", "detail": detail, "fix": fix});
    match gh.environment_branches(name) {
        Ok(EnvBranches::Custom(p)) if p.len() == 1 && p[0].0 == "main" && p[0].1 == "branch" => {
            json!({"name": name, "status": "ok", "detail": format!("only main deploys to {name}"), "fix": steps})
        }
        Ok(EnvBranches::Missing) => warn(
            format!(
                "environment {name} does not exist; a workflow run creates it with no branch limit"
            ),
            format!("Settings → Environments → New environment `{name}`, then {steps}"),
        ),
        Ok(EnvBranches::Any) => warn(
            format!("environment {name} has no deployment branch policy: any branch can deploy"),
            steps,
        ),
        Ok(EnvBranches::Protected) => warn(
            format!("environment {name} lets every protected branch deploy, not only main"),
            steps,
        ),
        Ok(EnvBranches::Custom(p)) if p.is_empty() => warn(
            format!("environment {name} allows no branch, not main"),
            steps,
        ),
        Ok(EnvBranches::Custom(p)) => {
            let list: Vec<String> = p
                .iter()
                .map(|(n, k)| {
                    if k == "branch" {
                        n.clone()
                    } else {
                        format!("{n} ({k})")
                    }
                })
                .collect();
            warn(
                format!("environment {name} allows {}, not only the branch main", list.join(", ")),
                steps,
            )
        }
        Ok(EnvBranches::Forbidden) => warn(
            format!("cannot verify environment {name} (403)"),
            ENV_GRANT_FIX.into(),
        ),
        Err(e) => warn(
            format!("cannot verify environment {name}: {}", e.message),
            "run `bosun doctor --remote` again; if it persists, check the token can read environments".into(),
        ),
    }
}

/// Check each workflow environment and fold the warnings into step 5. Returns `data.environments`.
fn apply_environments(items: &mut [Value], gh: &dyn Github, repo: &str, root: &Path) -> Value {
    let (names, unread) = workflow_environments(root);
    let mut envs: Vec<Value> = names
        .iter()
        .map(|n| environment_check(gh, repo, n))
        .collect();
    envs.extend(unread.iter().map(|(job, file)| json!({"job": job, "status": "warn",
        "detail": format!("cannot read the environment of job {job} in {file} (an expression, or no name)"),
        "fix": format!("name the environment literally in {file} (`environment: <name>`) so doctor can check it, or check its deployment branches by hand")})));
    let warned: Vec<&Value> = envs.iter().filter(|e| e["status"] == "warn").collect();
    let it = &mut items[4];
    let text = |k: &str| it[k].as_str().unwrap_or("").to_string();
    let (detail, fix) = (text("detail"), text("fix"));
    let join = |k: &str| {
        warned
            .iter()
            .filter_map(|e| e[k].as_str())
            .collect::<Vec<_>>()
            .join("; ")
    };
    if warned.is_empty() {
        if !names.is_empty() && it["status"] == "ok" {
            it["detail"] = json!(format!(
                "{detail}; only main deploys to {}",
                names.join(", ")
            ));
        }
    } else if it["status"] == "ok" {
        it["status"] = json!("warn");
        it["detail"] = json!(format!("{detail}; {}", join("detail")));
        it["fix"] = json!(join("fix"));
    } else {
        it["detail"] = json!(format!("{detail}; {}", join("detail")));
        it["fix"] = json!(format!("{fix}; then {}", join("fix")));
    }
    json!(envs)
}

const SETUP_SCRIPT_CAP: u64 = 64 * 1024;

/// The setup script's text, or why it cannot be shown: missing, a symlink, over the cap, unreadable.
fn read_setup_script(root: &Path) -> (Option<String>, Option<String>) {
    use std::io::Read;
    let path = root.join("docs/setup-script.sh");
    let bad = |m: String| (None, Some(format!("docs/setup-script.sh {m}")));
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return bad("is missing".into()),
        Err(e) => return bad(format!("is not readable: {e}")),
    };
    if meta.file_type().is_symlink() {
        return bad("is a symlink".into());
    }
    let mut text = String::new();
    let read = std::fs::File::open(&path)
        .and_then(|f| f.take(SETUP_SCRIPT_CAP + 1).read_to_string(&mut text));
    match read {
        Err(e) => bad(format!("is not readable: {e}")),
        Ok(_) if text.len() as u64 > SETUP_SCRIPT_CAP => bad("is over 64 KiB".into()),
        Ok(_) => (Some(text), None),
    }
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
    // The app acts as the owner's login, so GitHub's `performed_via_github_app` is the sign.
    if issues
        .iter()
        .any(|i| app_user(&i.user) || app_user(&i.via_app))
    {
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
        if gh
            .list_comments(n)?
            .iter()
            .any(|c| app_user(&c.user) || c.via_app.iter().flatten().any(|a| app_user(a)))
        {
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

/// In `[checks] mode = "checks"`: the gate contexts the live rulesets do not pin to the App, and
/// a sign that the token may hold write scopes it should not (spec §15, Residuals).
fn pin_warnings(gh: &dyn Github, ctx: &Ctx, branch: &str) -> Vec<String> {
    let Some(app_id) = ctx
        .config
        .checks
        .app_id
        .filter(|_| crate::github::gate::checks_mode())
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match gh.ruleset_pins(branch) {
        Ok(Some(pins)) => {
            let unpinned: Vec<&str> = ["risk-check", "review-verdict", "main-open"]
                .into_iter()
                .filter(|g| !pins.iter().any(|(c, id)| c == g && *id == Some(app_id)))
                .collect();
            if !unpinned.is_empty() {
                out.push(format!(
                    "{unpinned:?} not pinned to the gate App (integration_id {app_id}) in the rulesets: a status or another App's run would satisfy them; pin them in hoist.toml and run `hoist apply --area github`"
                ));
            }
        }
        Ok(None) => out.push("cannot read the rulesets' pins (403)".into()),
        Err(e) => out.push(format!("cannot read the rulesets' pins: {}", e.message)),
    }
    if gh.exists("/actions/secrets").ok().flatten() == Some(true) {
        out.push("this token can list the Actions secrets: it may hold write scopes an agent must not have (owner check H7)".into());
    }
    out
}

/// Mark `it` as a warning with `note` added, unless it is already missing (then only the note).
fn warn_on(it: &mut Value, note: &str) {
    let detail = format!("{}; {note}", it["detail"].as_str().unwrap_or(""));
    it["detail"] = json!(detail);
    if it["status"] == "ok" {
        it["status"] = json!("warn");
    }
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
/// Without a release setup, release items go to `skipped` and fail no step, unless hoist itself
/// reports release, npm or crates items: then release is configured and step 10 becomes
/// `release_step` (`data.hoist.release_inferred`).
fn apply_hoist(items: &mut [Value], root: &Path, release: bool, release_step: Value) -> Value {
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
    let inferred = !release
        && report["items"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|it| publish_evidence(it["area"].as_str().unwrap_or("")));
    if inferred {
        items[9] = release_step;
    }
    let release = release || inferred;
    let (mut drift, mut unknown, mut notes) = (Vec::new(), Vec::new(), Vec::new());
    let mut skipped = Vec::new();
    let mut per_step: std::collections::BTreeMap<usize, StepFindings> = [3, 5, 10]
        .into_iter()
        .map(|s| (s, StepFindings::default()))
        .collect();
    for it in report["items"].as_array().into_iter().flatten() {
        let s = |k: &str| it[k].as_str().unwrap_or("").to_string();
        if !release && release_item(&s("area"), &s("kind"), &s("key")) {
            if s("action") != "ok" {
                skipped.push(describe(it));
            }
            continue;
        }
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
    json!({"exit_code": exit_json, "drift": drift, "unknown": unknown, "notes": notes, "skipped": skipped, "release_inferred": inferred})
}

/// The plugins bosun's repos enable: (plugin name, pin name in the setup script). Both come from
/// the bosun-hosted marketplace `keelstack`, which pins each plugin's source commit.
const PLUGINS: [(&str, &str); 2] = [
    ("superpowers", "SUPERPOWERS_SHA"),
    ("caveman", "CAVEMAN_SHA"),
];
const MARKET: &str = "keelstack";
const MARKET_PIN: &str = "KEELSTACK_MARKETPLACE_REF";
const SWITCH: &str = "switch to the keelstack marketplace (`@keelstack`; hull repos render it)";

fn script_pin(script: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}=");
    script
        .lines()
        .find_map(|l| l.strip_prefix(prefix.as_str()))
        .map(|v| {
            let v = v.trim();
            match v.chars().next().filter(|c| matches!(c, '"' | '\'')) {
                Some(q) => v[1..].split(q).next().unwrap_or("").to_string(),
                None => v.split(" #").next().unwrap_or("").trim().to_string(),
            }
        })
        .filter(|v| !v.is_empty())
}

fn is_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Compare the plugin settings, the setup script's pins, the installed plugins and (when the repo
/// hosts it) the marketplace file. Pure: returns (problem, fix) pairs; every drift is a problem.
/// `settings` is `.claude/settings.json`, `script` the setup script text, `installed` the
/// installed_plugins.json (None = absent), `marketplace` `.claude-plugin/marketplace.json`.
fn plugin_findings(
    settings: Option<&Value>,
    script: Option<&str>,
    installed: Option<&Value>,
    marketplace: Option<&Value>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut add = |p: String, f: String| out.push((p, f));
    let pin_of = |name: &str| script.and_then(|s| script_pin(s, name));
    let settings_fix = format!(
        ".claude/settings.json: set extraKnownMarketplaces.{MARKET}.source to {{\"source\":\"github\",\"repo\":\"keelstackhq/bosun\",\"ref\":\"<{MARKET_PIN}>\"}} and enable superpowers@{MARKET} and caveman@{MARKET} in enabledPlugins; {SWITCH}"
    );
    let mut mref = None;
    match settings {
        None => add(
            ".claude/settings.json is missing or not JSON".into(),
            settings_fix.clone(),
        ),
        Some(s) => {
            let src = &s["extraKnownMarketplaces"][MARKET]["source"];
            if !src.is_null()
                && (src["source"] != "github"
                    || !src["repo"]
                        .as_str()
                        .is_some_and(|r| r.eq_ignore_ascii_case("keelstackhq/bosun")))
            {
                add(
                    format!("marketplace {MARKET} source is not github keelstackhq/bosun"),
                    settings_fix.clone(),
                );
            }
            match src["ref"].as_str() {
                None => add(
                    format!(".claude/settings.json has no {MARKET} marketplace ref"),
                    settings_fix.clone(),
                ),
                // Claude Code clones a marketplace with `--branch`: a sha (or nothing) fails.
                Some(r) if r.is_empty() || is_sha(r) => add(
                    format!("marketplace {MARKET} ref {r:?} is not a branch or tag"),
                    format!(
                        ".claude/settings.json: set extraKnownMarketplaces.{MARKET}.source.ref to {MARKET_PIN} of docs/setup-script.sh (a branch or tag, e.g. main)"
                    ),
                ),
                Some(r) => mref = Some(r),
            }
            for (name, _) in PLUGINS {
                if s["enabledPlugins"][format!("{name}@{MARKET}")] != true {
                    add(
                        format!("{name}@{MARKET} is not enabled in .claude/settings.json"),
                        format!(
                            ".claude/settings.json: set enabledPlugins.\"{name}@{MARKET}\" to true; {SWITCH}"
                        ),
                    );
                }
            }
        }
    }
    for pin in [MARKET_PIN, PLUGINS[0].1, PLUGINS[1].1] {
        if pin_of(pin).is_none() {
            let value = if pin == MARKET_PIN {
                "<branch, e.g. main>"
            } else {
                "<40 hex commit>"
            };
            add(
                format!("docs/setup-script.sh has no {pin}"),
                format!("docs/setup-script.sh: add a line {pin}={value}"),
            );
        }
    }
    if let (Some(r), Some(pin)) = (mref, pin_of(MARKET_PIN))
        && r != pin
    {
        add(
            format!("marketplace {MARKET} ref is {r} but {MARKET_PIN} is {pin}"),
            format!(
                ".claude/settings.json: set extraKnownMarketplaces.{MARKET}.source.ref to {pin}, or move {MARKET_PIN} in docs/setup-script.sh in a PR"
            ),
        );
    }
    match installed {
        None => add(
            "installed_plugins.json is missing ($CLAUDE_CONFIG_DIR or $HOME/.claude/plugins)"
                .into(),
            format!(
                "run the setup script (docs/setup-script.sh) so `claude plugin install superpowers@{MARKET} caveman@{MARKET}` writes installed_plugins.json"
            ),
        ),
        Some(inst) => {
            for (name, ppin) in PLUGINS {
                let id = format!("{name}@{MARKET}");
                let entry = &inst["plugins"][id.as_str()];
                let sha = entry[0]["gitCommitSha"]
                    .as_str()
                    .or_else(|| entry["gitCommitSha"].as_str());
                match (sha, pin_of(ppin)) {
                    (None, _) => add(
                        format!("{id} is not installed"),
                        format!(
                            "run `claude plugin install {id} --scope user` (docs/setup-script.sh does it); old entries such as {name}@superpowers-marketplace do not count: {SWITCH}"
                        ),
                    ),
                    (Some(sha), Some(pin)) if sha != pin => add(
                        format!("installed {id} is at {sha} but {ppin} is {pin}"),
                        format!(
                            "run `claude plugin update {id}` until it is at {pin}, or move {ppin} in docs/setup-script.sh in a PR"
                        ),
                    ),
                    _ => {}
                }
            }
        }
    }
    if let Some(m) = marketplace {
        for (name, ppin) in PLUGINS {
            let src = m["plugins"]
                .as_array()
                .and_then(|a| a.iter().find(|p| p["name"] == name))
                .and_then(|p| p["source"]["sha"].as_str());
            if let (Some(sha), Some(pin)) = (src, pin_of(ppin))
                && sha != pin
            {
                add(
                    format!(
                        ".claude-plugin/marketplace.json pins {name} at {sha} but {ppin} is {pin}"
                    ),
                    format!(
                        ".claude-plugin/marketplace.json: set the {name} source.sha to {pin}, or move {ppin} in docs/setup-script.sh to {sha}"
                    ),
                );
            }
        }
    }
    out
}

/// Step 13: the plugin settings, the setup script's pins, the installed plugins and the
/// marketplace file agree. Local only: no network.
fn plugins_item(root: &Path, script: Option<&str>) -> Value {
    let json_of = |t: String| serde_json::from_str::<Value>(&t).ok();
    let settings = read(root, ".claude/settings.json").and_then(json_of);
    let marketplace = read(root, ".claude-plugin/marketplace.json").and_then(json_of);
    let config = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".claude")));
    let installed = config
        .and_then(|c| std::fs::read_to_string(c.join("plugins/installed_plugins.json")).ok())
        .and_then(json_of);
    // The setup script text may be absent or rejected by step 6; its pins are read from disk.
    let raw = script
        .map(str::to_string)
        .or_else(|| read(root, "docs/setup-script.sh"));
    let found = plugin_findings(
        settings.as_ref(),
        raw.as_deref(),
        installed.as_ref(),
        marketplace.as_ref(),
    );
    let problem = (!found.is_empty()).then(|| {
        found
            .iter()
            .map(|(p, _)| p.as_str())
            .collect::<Vec<_>>()
            .join("; ")
    });
    let fix = if found.is_empty() {
        format!(
            "keep .claude/settings.json, docs/setup-script.sh and the installed plugins on the {MARKET} pins"
        )
    } else {
        found
            .iter()
            .map(|(_, f)| f.as_str())
            .collect::<Vec<_>>()
            .join(" | ")
    };
    item(
        13,
        "plugins",
        problem,
        format!(
            "the {MARKET} marketplace, its plugins, the setup script pins and the installed plugins agree"
        ),
        &fix,
    )
}

/// A set `BOSUN_GITHUB_API` must be an http(s) URL; otherwise doctor would blame the token.
fn check_api_url() -> Result<(), BosunError> {
    let Ok(raw) = std::env::var("BOSUN_GITHUB_API") else {
        return Ok(());
    };
    let ok = raw.parse::<ureq::http::Uri>().is_ok_and(|u| {
        matches!(u.scheme_str(), Some("http" | "https")) && u.host().is_some_and(|h| !h.is_empty())
    });
    if ok {
        return Ok(());
    }
    Err(BosunError::error(
        format!("BOSUN_GITHUB_API is not a valid URL: {raw}"),
        "unset BOSUN_GITHUB_API, or set it to an http(s) URL such as https://api.github.com, then rerun `bosun doctor`",
    ))
}

pub fn run(dir: Option<&Path>, remote: bool) -> CmdResult {
    let ctx = Ctx::load(dir)?;
    // Without a token the GitHub steps are skipped; the local steps still run.
    check_api_url()?;
    let live_gh = LiveGithub::new(&ctx.repo.owner, &ctx.repo.name).ok();
    let gh: Option<&dyn Github> = live_gh.as_ref().map(|g| g as &dyn Github);
    let offline = gh.is_none();
    let asked_remote = remote;
    let remote = remote && !offline;
    let ctx = &ctx;
    let root = ctx.dir.as_path();
    let mut items = Vec::new();

    let repo = gh.map_or(Ok(None), |g| g.repo_info());
    items.push(if gh.is_none() {
        no_token(1, "repo readable")
    } else {
        item(
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
        )
    });

    let seen = gh.map_or(Ok(false), claude_app_seen);
    items.push(if gh.is_none() {
        no_token(2, "Claude GitHub App")
    } else {
        item(
        2,
        "Claude GitHub App",
        match &seen {
            Ok(true) => None,
            Ok(false) => Some(
                "no PR, issue or comment by a `claude` login or via the `claude` app in the last 50 issues and PRs".into(),
            ),
            Err(e) => Some(e.message.clone()),
        },
        "a `claude` login has written here".into(),
        "install the app from https://github.com/apps/claude and grant it this repo",
    )
    });

    let hoist_text = read(root, "hoist.toml");
    let hoist = hoist_text
        .as_ref()
        .and_then(|t| t.parse::<toml::Table>().ok());
    // Strings and `{ context, app | integration_id }` tables (spec §15). A key that is there but
    // cannot be read would silently fall back to the defaults, so it is a problem of its own.
    let listed: Vec<String> = (hoist_text.as_deref())
        .and_then(checks::hoist_required)
        .unwrap_or_default();
    let unreadable = hoist.as_ref().is_some_and(|h| {
        h.get("github")
            .and_then(|g| g.get("required_checks"))
            .is_some()
    }) && listed.is_empty();
    let jobs = workflow_jobs(root);
    let has_job = |j: &str| jobs.iter().any(|x| x == j);
    let mut absent: Vec<&str> = checks::DEFAULT_REQUIRED.to_vec();
    if has_job("bosun-ok") {
        absent.push("bosun-ok");
    }
    absent.retain(|c| !listed.iter().any(|l| l == c));
    // What the default branch enforces. A 403 proves nothing (a private repo on a free plan has
    // no rulesets), so it only adds a note, like the `unknown` items of `hoist check`.
    let live = match (&repo, gh) {
        (Ok(Some(info)), Some(gh)) => Some(gh.required_status_checks(&info.default_branch)),
        _ => None,
    };
    // A check counts as unenforced only when both endpoints were readable; with one 403, the
    // checks the other does not show may sit behind it, so they only add a note.
    let not_on =
        |on: &Vec<String>| -> Vec<&String> { listed.iter().filter(|c| !on.contains(c)).collect() };
    let unenforced: Vec<&String> = match &live {
        Some(Ok(Live::Checks(on))) => not_on(on),
        _ => Vec::new(),
    };
    let live_note = match &live {
        Some(Ok(Live::Partial(on, unreadable))) if !not_on(on).is_empty() => Some(format!(
            "cannot verify the enforced checks: {unreadable} answered 403 and the rest does not \
             require {:?}; give the token Administration: read to check",
            not_on(on)
        )),
        Some(Ok(Live::Forbidden)) => Some(
            "cannot verify the enforced checks (403: a private repo on a free plan has no \
             rulesets; make it public or use a paid plan, or give the token Administration: read)"
                .to_string(),
        ),
        Some(Err(e)) => Some(format!("cannot verify the enforced checks: {}", e.message)),
        _ => None,
    };
    let step3_fix = if absent.is_empty() && !unenforced.is_empty() {
        format!("run `hoist apply --area github` (an admin) so the ruleset requires {unenforced:?}")
    } else if absent.is_empty() {
        HOIST_FIX.to_string()
    } else {
        format!("add {absent:?} to hoist.toml [github] required_checks, then {HOIST_FIX}")
    };
    let step3 = item(
        3,
        "required checks",
        if hoist.is_none() {
            Some("hoist.toml is missing or not valid TOML".into())
        } else if unreadable {
            Some("hoist.toml [github] required_checks cannot be read: use strings or { context, app } / { context, integration_id } tables".into())
        } else if !absent.is_empty() {
            Some(format!(
                "hoist.toml [github] required_checks lacks {absent:?}"
            ))
        } else if !has_job("ci-ok") {
            Some("no .github/workflows/*.yml has a `ci-ok` job".into())
        } else if !unenforced.is_empty() {
            Some(format!(
                "hoist.toml lists {unenforced:?}, but the default branch does not require them"
            ))
        } else {
            None
        }
        .map(|p| join_note(p, &live_note)),
        join_note(
            "hoist.toml lists the required checks, a workflow has the ci-ok job and the default branch enforces them"
                .into(),
            &live_note,
        ),
        &step3_fix,
    );
    // Without a token only the local findings count; an unverified pass is skipped.
    items.push(if gh.is_none() && step3["status"] == "ok" {
        no_token(3, "required checks")
    } else {
        step3
    });

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
    let label = gh.map_or(Ok(true), |g| g.label_exists(HALT_LABEL));
    let halt = gh.map_or(Ok(None), halt_reason);
    items.push(if gh.is_none() {
        no_token(5, "kill switch")
    } else {
        item(
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
            &format!(
                "gh label create bosun:halt --color B60205 --repo {}",
                ctx.repo
            ),
        )
    });

    let version = read(root, ".bosun/version").map(|v| v.trim().to_string());
    let (script, script_problem) = read_setup_script(root);
    let mut setup = item(
        6,
        "setup script and version",
        if script_problem.is_some() {
            script_problem
        } else if version.is_none() {
            Some(".bosun/version is missing".into())
        } else {
            None
        },
        format!(
            "docs/setup-script.sh exists (content below) for the shared cloud environment \"keelstack\"; .bosun/version is {}",
            version.clone().unwrap_or_default()
        ),
        "run `bosun setup-snippet` and paste its script once as the setup script of the shared cloud environment \"keelstack\" (claude.ai/code > Environments); it runs each checkout's docs/setup-script.sh (the content field), so keep that file and .bosun/version",
    );
    setup["environment"] = json!("keelstack");
    setup["run"] = json!("bosun setup-snippet");
    setup["content"] = json!(script);
    items.push(setup);

    let channel = ctx
        .config
        .slack
        .as_ref()
        .map(|s| s.channel.trim().to_string())
        .unwrap_or_default();
    let slack_set = ctx.config.slack.is_some();
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
    if !slack_set {
        slack_item = skipped(
            7,
            "Slack channel",
            "Slack is optional and not configured (no [slack] table in .bosun/bosun.toml); notify is skipped",
            "to use Slack: create the channel, connect Claude Tag, and add [slack] channel to .bosun/bosun.toml",
        );
    }
    items.push(slack_item);

    let reg = gh.map_or(Ok(None), registry::load);
    items.push(if gh.is_none() {
        no_token(8, "registry issue")
    } else {
        item(
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
        "run `bosun adopt` (with GITHUB_TOKEN set) or `bosun join`; either creates the registry issue",
    )
    });

    items.push(if gh.is_none() {
        no_token(9, "auto-merge")
    } else {
        item(
            9,
            "auto-merge",
            match &repo {
                Ok(Some(r)) if r.allow_auto_merge => None,
                Ok(Some(_)) => Some("allow_auto_merge is off".into()),
                _ => Some("the repo is not readable".into()),
            },
            "allow_auto_merge is on".into(),
            HOIST_FIX,
        )
    });

    let release = gh.map_or(Ok(None), |g| g.latest_release());
    let release_set = release_configured(root, hoist.as_ref(), matches!(release, Ok(Some(_))));
    let release_step = item(
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
        "run `hoist release bootstrap` from a checkout of main with an admin token",
    );
    items.push(if release_set && gh.is_none() {
        no_token(10, "release")
    } else if release_set {
        release_step.clone()
    } else {
        skipped(
            10,
            "release",
            "release is optional and not configured (no [release] or [publish] in hoist.toml, no release workflow or environment, no publishable Cargo or npm package, no release yet)",
            "to publish: add [release] to hoist.toml, then run `hoist release bootstrap`",
        )
    });

    let routines_ok = std::env::var("BOSUN_ROUTINES_OK").is_ok_and(|v| v.trim() == "1");
    items.push(item(
        11,
        "Routines",
        (!routines_ok).then(|| "bosun cannot see Routines; create them from the prompts in data.routines".into()),
        "BOSUN_ROUTINES_OK=1: the owner confirmed the Routines in data.routines".into(),
        if slack_set {
            "create the janitor, curator and notify Routines from data.routines, then set BOSUN_ROUTINES_OK=1"
        } else {
            "create the janitor and curator Routines from data.routines, then set BOSUN_ROUTINES_OK=1"
        },
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

    items.push(plugins_item(root, script.as_deref()));

    // Step 14: the CI-minute rules (docs/ci-minutes.md), the same check as `bosun gate ci-minutes`.
    let (_, minutes) = crate::cli::ci_minutes::scan(root);
    items.push(item(
        14,
        "CI minutes",
        minutes.first().map(|f| {
            format!(
                "{} CI-minute finding(s); first: {} {}{}",
                minutes.len(),
                f.file,
                f.rule,
                f.job
                    .as_ref()
                    .map_or(String::new(), |j| format!(" (job {j})"))
            )
        }),
        "every pull_request workflow skips drafts, runs on ready_for_review and cancels older runs"
            .into(),
        &minutes.first().map_or_else(
            || "run `bosun gate ci-minutes`".to_string(),
            |f| format!("{}; `bosun gate ci-minutes` lists every finding", f.fix),
        ),
    ));

    if let Some(slack) = ctx.config.slack.as_ref() {
        let seat = gh.map_or(Ok(None), |g| registry::seat_of(g, &ctx.session, ctx.now));
        let seat_ok = seat.as_ref().ok().cloned().flatten();
        let mut tag = item(
            15,
            "claude-tag",
            None,
            match &seat_ok {
                Some(s) => format!(
                    "this session ({}) sits in {} as the {} seat{}",
                    ctx.session,
                    s.channel,
                    s.role,
                    s.thread_ts
                        .as_ref()
                        .map_or(String::new(), |t| format!(" of thread {t}"))
                ),
                None => String::new(),
            },
            "",
        );
        if gh.is_none() {
            tag = no_token(15, "claude-tag");
        } else if seat_ok.is_none() {
            // No seat is normal for a session outside Claude Tag: skipped, not a warning. A
            // failed registry read is a warning.
            tag["status"] = json!(if seat.is_err() { "warn" } else { "skipped" });
            tag["detail"] = json!(match &seat {
                Err(e) => format!("the registry could not be read: {}", e.message),
                Ok(_) => "no Claude Tag seat for this session".to_string(),
            });
            tag["fix"] = json!(format!(
                "in a Claude Tag session run `bosun join --origin slack --slack-channel {} [--slack-thread <ts>]`",
                slack.channel.trim()
            ));
        }
        items.push(tag);
    }
    let pat = ["GITHUB_TOKEN", "GH_TOKEN"].iter().any(|k| {
        std::env::var(k)
            .is_ok_and(|v| v.trim().starts_with("ghp_") || v.trim().starts_with("github_pat_"))
    });
    let cloud = crate::core::approve::in_cloud(
        std::env::var("CCR_AGENT_PROXY_ENABLED").ok().as_deref(),
        std::env::var("CLAUDE_CODE_REMOTE_ENVIRONMENT_TYPE")
            .ok()
            .as_deref(),
    );
    if pat && cloud {
        let mut it = item(
            16,
            "github-token",
            None,
            String::new(),
            "unset GITHUB_TOKEN and GH_TOKEN in the environment's settings; the session needs no token",
        );
        it["status"] = json!("warn");
        it["detail"] = json!(
            "a personal access token is in the environment; remove it: cloud and Claude Tag sessions reach GitHub through the Claude GitHub App proxy"
        );
        items.push(it);
    }

    items.push(match super::pin::update_check(root) {
        Check::Skipped(why, fix) | Check::Failed(why, fix) => skipped(17, "update", &why, &fix),
        Check::Current => item(
            17,
            "update",
            None,
            "pins and binary match npm latest".into(),
            "",
        ),
        Check::Behind(u) => {
            let mut parts = Vec::new();
            if !u.moves.is_empty() {
                parts.push(format!("tool pins behind npm: {}", u.moves.join(", ")));
            }
            let mut fix = if u.channel {
                "splice update --channel bosun".to_string()
            } else {
                "bosun pin --latest --pr".to_string()
            };
            if let Some(v) = &u.cli {
                let cmd = format!("npm i -g @keelstackhq/bosun@{v}");
                let when = if u.moves.is_empty() {
                    fix = cmd.clone();
                    ""
                } else {
                    " after the pin PR merges"
                };
                parts.push(format!(
                    "this machine runs bosun {}; run `{cmd}`{when}",
                    env!("CARGO_PKG_VERSION")
                ));
            }
            let mut it = item(17, "update", None, String::new(), &fix);
            it["status"] = json!("warn");
            it["detail"] = json!(parts.join("; "));
            it
        }
    });

    let hoist_data = remote.then(|| apply_hoist(&mut items, root, release_set, release_step));
    if let (true, Some(gh), Ok(Some(info))) = (remote, gh, &repo) {
        let warnings = pin_warnings(gh, ctx, &info.default_branch);
        if let Some(step3) = items.iter_mut().find(|i| i["step"] == 3) {
            warnings.iter().for_each(|w| warn_on(step3, w));
        }
    }
    let envs = gh
        .filter(|_| remote)
        .map(|g| apply_environments(&mut items, g, &ctx.repo.to_string(), root));
    let missing = items.iter().filter(|i| i["status"] == "missing").count();
    if offline {
        // Nothing on GitHub was checked, so this can never be a full pass.
        let mut names: Vec<String> = items
            .iter()
            .filter(|i| i["status"] == "skipped" && i["fix"] == NO_TOKEN_FIX)
            .filter_map(|i| i["name"].as_str().map(String::from))
            .collect();
        if asked_remote {
            names.push("hoist check".into());
            names.push("workflow environments".into());
        }
        items.push(json!({
            "step": 0, "name": "remote checks", "status": "skipped",
            "detail": format!("not run, no GITHUB_TOKEN or GH_TOKEN: {}", names.join(", ")),
            "fix": NO_TOKEN_FIX,
        }));
    }
    let mut data = json!({"items": items, "missing": missing, "offline": offline, "routines": routines(slack_set)});
    if !slack_set {
        data["notify"] = json!({"skipped": true, "reason": "no [slack] table in .bosun/bosun.toml: the janitor skips notify and there is no notify Routine"});
    }
    if let Some(h) = hoist_data {
        data["hoist"] = h;
    }
    if let Some(e) = envs {
        data["environments"] = e;
    }
    let first = data["items"]
        .as_array()
        .and_then(|a| a.iter().find(|i| i["status"] == "missing"))
        .map(|i| json!({"step": i["step"], "fix": i["fix"]}));
    if let Some(n) = first {
        data["next_step"] = n;
    } else if offline {
        data["next_step"] = json!({"step": 0, "fix": NO_TOKEN_FIX});
    }
    if offline {
        let message = if missing > 0 {
            format!("{missing} checklist steps missing; GitHub not checked (no token)")
        } else {
            "0 checklist steps missing; GitHub not checked (no token)".into()
        };
        let fix = if missing > 0 {
            "do the missing steps in data.items, starting with data.next_step, then export GITHUB_TOKEN (repo and workflow scopes) and rerun `bosun doctor`"
        } else {
            NO_TOKEN_FIX
        };
        return Err(BosunError::needs_human(message, fix).with_data(data));
    }
    if missing > 0 {
        return Err(BosunError::needs_human(
            format!("{missing} checklist steps missing"),
            "do the missing steps in data.items, starting with data.next_step",
        )
        .with_data(data));
    }
    Ok(Outcome::data(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    const M1: &str = "main";
    const P1: &str = "22f0b463e7a5e579904dd6211a520321035526f7";
    const C: &str = "a9cc1f7b82598852e8c6b8e599dde6ac2491b26e";

    fn script() -> String {
        format!("KEELSTACK_MARKETPLACE_REF={M1}\nSUPERPOWERS_SHA={P1}\nCAVEMAN_SHA={C}\n")
    }

    fn settings(r: &str, caveman_on: bool) -> Value {
        json!({
            "extraKnownMarketplaces": {"keelstack": {"source": {"source": "github", "repo": "keelstackhq/bosun", "ref": r}}},
            "enabledPlugins": {"superpowers@keelstack": true, "caveman@keelstack": caveman_on}
        })
    }

    fn installed(sp: &str, cv: &str) -> Value {
        json!({"plugins": {
            "superpowers@keelstack": [{"gitCommitSha": sp}],
            "caveman@keelstack": [{"gitCommitSha": cv}]
        }})
    }

    fn market(sp: &str, cv: &str) -> Value {
        json!({"plugins": [
            {"name": "superpowers", "source": {"source": "github", "repo": "obra/superpowers", "sha": sp}},
            {"name": "caveman", "source": {"source": "github", "repo": "akramsystems/caveman", "sha": cv}}
        ]})
    }

    fn run(s: Option<&Value>, sc: Option<&str>, i: Option<&Value>, m: Option<&Value>) -> String {
        plugin_findings(s, sc, i, m)
            .iter()
            .map(|(p, f)| format!("{p} => {f}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn agreeing_settings_script_install_and_marketplace_have_no_problem() {
        let out = run(
            Some(&settings(M1, true)),
            Some(&script()),
            Some(&installed(P1, C)),
            Some(&market(P1, C)),
        );
        assert_eq!(out, "");
        // The marketplace file is optional.
        let out = run(
            Some(&settings(M1, true)),
            Some(&script()),
            Some(&installed(P1, C)),
            None,
        );
        assert_eq!(out, "");
    }

    #[test]
    fn a_missing_install_is_a_problem() {
        let out = run(Some(&settings(M1, true)), Some(&script()), None, None);
        assert!(out.contains("installed_plugins.json is missing"), "{out}");
    }

    #[test]
    fn an_old_marketplace_entry_does_not_count() {
        let old = json!({"plugins": {"superpowers@superpowers-marketplace": [{"gitCommitSha": P1}],
            "caveman@keelstack": [{"gitCommitSha": C}]}});
        let out = run(Some(&settings(M1, true)), Some(&script()), Some(&old), None);
        assert!(
            out.contains("superpowers@keelstack is not installed"),
            "{out}"
        );
        assert!(out.contains("@keelstack"), "{out}");
    }

    #[test]
    fn a_disabled_plugin_is_a_problem() {
        let out = run(
            Some(&settings(M1, false)),
            Some(&script()),
            Some(&installed(P1, C)),
            None,
        );
        assert!(out.contains("caveman@keelstack is not enabled"), "{out}");
        assert!(out.contains(".claude/settings.json"), "{out}");
    }

    #[test]
    fn a_missing_marketplace_or_a_bad_ref_is_a_problem() {
        let out = run(
            Some(&json!({})),
            Some(&script()),
            Some(&installed(P1, C)),
            None,
        );
        assert!(out.contains("no keelstack marketplace ref"), "{out}");
        for bad in ["5c9072877417873fee42be1805aa776a453ec394", ""] {
            let out = run(
                Some(&settings(bad, true)),
                Some(&script()),
                Some(&installed(P1, C)),
                None,
            );
            assert!(out.contains("branch or tag"), "{out}");
        }
        let out = run(None, Some(&script()), Some(&installed(P1, C)), None);
        assert!(out.contains("settings.json is missing"), "{out}");
    }

    #[test]
    fn a_ref_off_the_script_pin_is_a_problem() {
        let other = "release";
        let out = run(
            Some(&settings(other, true)),
            Some(&script()),
            Some(&installed(P1, C)),
            None,
        );
        assert!(out.contains("KEELSTACK_MARKETPLACE_REF"), "{out}");
        assert!(out.contains(M1), "{out}");
    }

    #[test]
    fn a_foreign_marketplace_source_is_a_problem() {
        let want = "keelstackhq/bosun";
        for src in [
            json!({"source": "github", "repo": "evil/fork", "ref": M1}),
            json!({"source": "url", "url": "https://x.test/y.git", "ref": M1}),
            json!({"source": "git", "url": "https://x.test/y.git", "ref": M1}),
        ] {
            let mut s = settings(M1, true);
            s["extraKnownMarketplaces"]["keelstack"]["source"] = src;
            let out = run(Some(&s), Some(&script()), Some(&installed(P1, C)), None);
            assert!(out.contains("not github keelstackhq/bosun"), "{out}");
            assert!(out.contains(want), "{out}");
        }
        let mut s = settings(M1, true);
        s["extraKnownMarketplaces"]["keelstack"]["source"]["repo"] = json!("KeelstackHQ/Bosun");
        assert_eq!(
            run(Some(&s), Some(&script()), Some(&installed(P1, C)), None),
            ""
        );
    }

    #[test]
    fn script_pin_strips_comments_and_quotes() {
        let sc = "CAVEMAN_SHA=abc # note\nA=\"def\" # x\nB='ghi'\nC=\"j#k\"\n";
        assert_eq!(script_pin(sc, "CAVEMAN_SHA").as_deref(), Some("abc"));
        assert_eq!(script_pin(sc, "A").as_deref(), Some("def"));
        assert_eq!(script_pin(sc, "B").as_deref(), Some("ghi"));
        assert_eq!(script_pin(sc, "C").as_deref(), Some("j#k"));
    }

    #[test]
    fn a_missing_script_pin_is_a_problem() {
        let sc = format!("KEELSTACK_MARKETPLACE_REF={M1}\nSUPERPOWERS_SHA={P1}\n");
        let out = run(
            Some(&settings(M1, true)),
            Some(&sc),
            Some(&installed(P1, C)),
            None,
        );
        assert!(out.contains("no CAVEMAN_SHA"), "{out}");
        let out = run(
            Some(&settings(M1, true)),
            None,
            Some(&installed(P1, C)),
            None,
        );
        assert_eq!(
            out.matches("docs/setup-script.sh has no").count(),
            3,
            "{out}"
        );
    }

    #[test]
    fn an_installed_commit_off_its_pin_is_a_problem() {
        let out = run(
            Some(&settings(M1, true)),
            Some(&script()),
            Some(&installed("abc", C)),
            None,
        );
        assert!(out.contains("SUPERPOWERS_SHA"), "{out}");
    }

    #[test]
    fn a_marketplace_file_sha_off_the_script_pin_is_a_problem() {
        let other = "c".repeat(40);
        let out = run(
            Some(&settings(M1, true)),
            Some(&script()),
            Some(&installed(P1, C)),
            Some(&market(P1, &other)),
        );
        assert!(
            out.contains(".claude-plugin/marketplace.json pins caveman"),
            "{out}"
        );
    }
}
