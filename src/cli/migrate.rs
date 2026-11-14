//! `bosun migrate --to <mode>`: one governance PR that moves `[mode]` one rung (or straight to the
//! target with `--direct`), after a read-only survey, with the version and registry pins it needs.
//! `--finish` runs after the merge: join, then plan amend for each plan (both idempotent).
//! Never merges and never force-pushes (D13).

use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{JoinArgs, MigrateArgs, PlanAmendArgs, PlanCmd};
use crate::app::{App, default_branch, kill_switch};
use crate::config::{CONFIG_FILE, Config};
use crate::core::meta::parse_task_meta;
use crate::core::migrate::{
    BodyIn, MODE_MIN, below_min, body, branch_step, is_lighter, owner_steps, parse_mode,
    redact_credentials, semver_lt, set_mode_name, step,
};
use crate::core::mode::{BudgetFeature, ModeName};
use crate::core::pin::{get_line, keys};
use crate::github::{Github, IssueState, Pull};
use crate::output::{BosunError, CmdResult, Outcome};

const BRANCH: &str = "bosun/migrate-";
const VERSION_FILE: &str = ".bosun/version";
const REGISTRY_FILE: &str = ".bosun/registry";

/// Everything the survey read; every failure is a note, never an error.
struct Survey {
    version: Option<String>,
    pin_file: bool,
    pinned: Option<u64>,
    live: Vec<u64>,
    epics_unmarked: Vec<u64>,
    plans: Vec<String>,
    hull_lock: bool,
    hull_version: Option<String>,
    bosun_channel: bool,
    baseline: Result<Value, String>,
    notes: Vec<String>,
}

impl Survey {
    fn json(&self) -> Value {
        json!({
            "version": self.version,
            "binary": env!("CARGO_PKG_VERSION"),
            "registry": {"pinned": self.pinned, "live": self.live.first()},
            "epics_unmarked": self.epics_unmarked,
            "plans": self.plans,
            "hull": {"lock": self.hull_lock, "version": self.hull_version},
            "bosun_channel": self.bosun_channel,
            "baseline": self.baseline.as_ref().ok(),
            "notes": self.notes,
        })
    }
}

fn git(root: &Path, args: &[&str]) -> Result<String, BosunError> {
    Ok(git_raw(root, args, None)?.trim().to_string())
}

/// A git call that talks to the remote: killed after 60 s (`BOSUN_MIGRATE_GIT_TIMEOUT` seconds).
fn git_net(root: &Path, args: &[&str]) -> Result<String, BosunError> {
    let secs = std::env::var("BOSUN_MIGRATE_GIT_TIMEOUT")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(60);
    Ok(git_raw(root, args, Some(Duration::from_secs(secs)))?
        .trim()
        .to_string())
}

/// Run git in `root` with prompts off, stdout untrimmed. Anything quoted from git's stderr has
/// URL credentials removed.
fn git_raw(root: &Path, args: &[&str], limit: Option<Duration>) -> Result<String, BosunError> {
    let fail = |what: String| {
        BosunError::error(
            redact_credentials(&what),
            "fix the git error above, then rerun `bosun migrate`",
        )
    };
    let base = || {
        let mut c = Command::new("git");
        c.arg("-C")
            .arg(root)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_ASKPASS", "true")
            .stdin(Stdio::null());
        c
    };
    // A runner with no git identity still commits, as bosun.
    let has = |key: &str| {
        base()
            .args(["config", key])
            .output()
            .is_ok_and(|o| o.status.success())
    };
    let mut c = base();
    if !(has("user.email") && has("user.name")) {
        c.args([
            "-c",
            "user.name=bosun",
            "-c",
            "user.email=bosun@users.noreply.github.com",
        ]);
    }
    let mut child = c
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            BosunError::error(format!("cannot run git: {e}"), "install git, then rerun")
        })?;
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let err = drain(child.stderr.take().map(|p| Box::new(p) as _));
    let deadline = limit.map(|l| Instant::now() + l);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if deadline.is_some_and(|d| Instant::now() >= d) => {
                // A helper (ssh) may still hold the pipes: leave the readers behind.
                let _ = child.kill();
                let _ = child.wait();
                return Err(fail(format!(
                    "git {} timed out after {}s",
                    args.join(" "),
                    limit.map_or(0, |l| l.as_secs())
                )));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(fail(format!("cannot wait for git: {e}"))),
        }
    };
    let (out, err) = (
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default(),
    );
    if !status.success() {
        return Err(fail(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&err).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out).to_string())
}

/// Run this binary with `args` in `root`: its exit code and the JSON object it printed. `Err`
/// says why there is none (cannot start, timed out, no JSON).
fn run_self(root: &Path, args: &[&str], limit: Duration) -> Result<(i32, Value), String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find this binary: {e}"))?;
    let mut child = Command::new(exe)
        .arg("-C")
        .arg(root)
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start `bosun {}`: {e}", args.join(" ")))?;
    let mut pipe = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(format!(
                    "`bosun {}` timed out after {}s",
                    args.join(" "),
                    limit.as_secs()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("cannot wait for `bosun {}`: {e}", args.join(" "))),
        }
    };
    let out = reader.join().unwrap_or_default();
    let v: Value = serde_json::from_slice(&out)
        .map_err(|_| format!("`bosun {}` printed no JSON object", args.join(" ")))?;
    Ok((status.code().unwrap_or(1), v))
}

fn error_text(v: &Value) -> String {
    v["error"]["message"]
        .as_str()
        .unwrap_or("no error message")
        .to_string()
}

fn error_fix(v: &Value) -> String {
    match v["error"]["fix"].as_str() {
        Some(f) if !f.trim().is_empty() => f.to_string(),
        _ => "see the error above, then rerun `bosun migrate`".to_string(),
    }
}

fn read(root: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(root.join(rel)).ok()
}

/// The `version = "x"` value of a TOML text (first one).
fn first_version(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let rest = l.trim().strip_prefix("version")?.trim_start();
        let v = rest.strip_prefix('=')?.trim().trim_matches('"');
        (!v.is_empty()).then(|| v.to_string())
    })
}

/// The numbers of the unmarked epics and the plan paths of the marked open ones.
fn epics(gh: &dyn Github) -> Result<(Vec<u64>, Vec<String>), BosunError> {
    const MARK: &str = "<!-- bosun:epic";
    let open = gh.list_issues(&["bosun:source/plan"], IssueState::Open)?;
    let mut plans: Vec<String> = Vec::new();
    for i in open.iter().filter(|i| !i.pull_request) {
        if let Some(p) = super::plan::epic_plan(i.body_text())
            && !plans.contains(&p)
        {
            plans.push(p);
        }
    }
    let mut unmarked = BTreeSet::new();
    for e in open
        .iter()
        .filter_map(|i| parse_task_meta(i.body_text()).and_then(|m| m.epic))
        .collect::<BTreeSet<u64>>()
    {
        let body = match open.iter().find(|i| i.number == e) {
            Some(i) => Some(i.body_text().to_string()),
            None => gh
                .get_issue(e)?
                .filter(|i| i.state == "open")
                .map(|i| i.body_text().to_string()),
        };
        if body.is_some_and(|b| !b.contains(MARK)) {
            unmarked.insert(e);
        }
    }
    Ok((unmarked.into_iter().collect(), plans))
}

fn baseline(root: &Path) -> Result<Value, String> {
    let secs = std::env::var("BOSUN_MIGRATE_METRICS_TIMEOUT")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(20);
    match run_self(
        root,
        &["metrics", "--days", "30"],
        Duration::from_secs(secs),
    )? {
        (0, v) => Ok(v["data"].clone()),
        (code, v) => Err(format!("metrics exited {code}: {}", error_text(&v))),
    }
}

fn survey(app: &App, with_baseline: bool) -> Survey {
    let root = app.ctx.dir.as_path();
    let mut s = Survey {
        version: read(root, VERSION_FILE).map(|v| v.trim().to_string()),
        pin_file: false,
        pinned: None,
        live: Vec::new(),
        epics_unmarked: Vec::new(),
        plans: Vec::new(),
        hull_lock: false,
        hull_version: None,
        bosun_channel: root.join(".splice/channels/bosun.toml").exists(),
        baseline: Err("not run".into()),
        notes: Vec::new(),
    };
    if let Some(t) = read(root, REGISTRY_FILE) {
        s.pin_file = true;
        s.pinned = t.trim().parse().ok();
        if s.pinned.is_none() {
            s.notes.push(format!(
                "{REGISTRY_FILE} is not an issue number: {:?}",
                t.trim()
            ));
        }
    }
    match app
        .gh
        .list_issues(&[crate::registry::LABEL], IssueState::Open)
    {
        Ok(v) => {
            s.live = v
                .iter()
                .filter(|i| !i.pull_request)
                .map(|i| i.number)
                .collect();
            s.live.sort_unstable();
        }
        Err(e) => s
            .notes
            .push(format!("cannot list the registry issues: {}", e.message)),
    }
    match epics(app.gh.as_ref()) {
        Ok((u, p)) => {
            s.epics_unmarked = u;
            s.plans = p;
        }
        Err(e) => s
            .notes
            .push(format!("cannot read the epics: {}", e.message)),
    }
    if let Some(t) = read(root, ".splice/lock.toml") {
        s.hull_lock = true;
        s.hull_version = first_version(&t);
    }
    if with_baseline {
        s.baseline = baseline(root);
        if let Err(why) = &s.baseline {
            s.notes.push(format!("baseline: unavailable ({why})"));
        }
    } else {
        s.baseline = Err("skipped".into());
    }
    s
}

fn compact(v: &Value) -> String {
    let t = v.to_string();
    if t.chars().count() > 600 {
        format!("{}...", t.chars().take(600).collect::<String>())
    } else {
        t
    }
}

/// The open migration PR of this repo, head `bosun/migrate-*` (a fork's PR of that name is not
/// ours).
fn find_open(gh: &dyn Github, repo: &str) -> Result<Option<Pull>, BosunError> {
    Ok(gh
        .list_pulls(None, IssueState::Open)?
        .into_iter()
        .find(|p| p.head_ref.starts_with(BRANCH) && p.head_repo.eq_ignore_ascii_case(repo)))
}

/// `bosun/migrate-<step>` is free, else one dated name.
fn free_branch(root: &Path, step: ModeName) -> Result<String, BosunError> {
    let taken = |b: &str| {
        git(
            root,
            &["rev-parse", "--verify", "-q", &format!("refs/heads/{b}")],
        )
        .is_ok()
            || git_net(root, &["ls-remote", "--exit-code", "--heads", "origin", b]).is_ok()
    };
    let plain = format!("{BRANCH}{}", step.as_str());
    let day = crate::time::Timestamp::now().to_string().replace('-', "");
    let dated = format!("{plain}-{}", &day[..8]);
    if !taken(&plain) {
        Ok(plain)
    } else if !taken(&dated) {
        Ok(dated)
    } else {
        Err(BosunError::conflict(
            format!("{plain} and {dated} already exist: they are left from earlier migrations"),
            "delete both names locally (`git branch -D <name>`) and on origin (the owner, on GitHub's Branches page), then rerun",
        ))
    }
}

fn approval(lighter: bool, pr: Option<(u64, &str)>) -> String {
    let what = match pr {
        Some((n, sha)) => format!("PR #{n} at head SHA {sha}"),
        None => "the migration PR at its head SHA".to_string(),
    };
    let weaken = if lighter {
        ", and R-weaken because the mode gets lighter"
    } else {
        ""
    };
    format!(
        "Owner: approve {what} through the chat form (repo, PR, head SHA). The PR is R-gov{weaken}; one approval covers the whole migration."
    )
}

fn then(pr: Option<u64>, step: ModeName, to: ModeName) -> Vec<String> {
    let n = pr.map_or("<n>".to_string(), |n| n.to_string());
    let mut v = vec![
        "run the gate loop (`bosun gate <g>` for each ready gate)".to_string(),
        format!("`bosun pr --pr {n} --ready`"),
        "after merge: `bosun migrate --finish`".to_string(),
    ];
    if step != to {
        v.push(format!(
            "after two weeks on {}, rerun `bosun migrate --to {}`",
            step.as_str(),
            to.as_str()
        ));
    }
    v
}

pub fn run(dir: Option<&Path>, a: &MigrateArgs) -> CmdResult {
    let app = App::load(dir)?;
    if !a.finish && !a.plan.is_empty() {
        return Err(BosunError::usage(
            "--plan only goes with --finish",
            "add --finish, or drop --plan",
        ));
    }
    kill_switch(app.gh.as_ref())?;
    if a.finish {
        return finish(dir, &app, a);
    }
    let to = a.to.as_deref().and_then(parse_mode).ok_or_else(|| {
        BosunError::usage(
            "--to is missing or not a mode",
            "pass --to gatekeeper|steady|booster|autonomous",
        )
    })?;
    let root = app.ctx.dir.clone();
    let from = app.ctx.config.mode.name.unwrap_or(ModeName::Gatekeeper);
    let next = step(from, to, a.direct);
    let lighter = is_lighter(from, next);
    let s = survey(&app, true);
    if s.pinned.is_none() && !s.pin_file && s.live.len() > 1 {
        return Err(BosunError::conflict(
            format!(
                "{} open issues carry the label {} and {REGISTRY_FILE} does not say which is live: {:?}",
                s.live.len(),
                crate::registry::LABEL,
                s.live
            ),
            format!(
                "the owner closes the extra registry issue, or tells the agent which one is live (then write its number to {REGISTRY_FILE} by hand), then rerun"
            ),
        )
        .with_data(json!({"registry": s.live})));
    }
    // What has to change.
    let need_pin = next != ModeName::Gatekeeper && below_min(s.version.as_deref());
    let need_registry = if s.pin_file {
        None
    } else {
        s.live.first().copied()
    };
    let old_toml = read(&root, CONFIG_FILE).unwrap_or_default();
    let new_toml = (next != from).then(|| set_mode_name(&old_toml, next));
    if let Some(t) = &new_toml {
        Config::parse(t, CONFIG_FILE).map_err(|e| {
            BosunError::error(
                format!(
                    "{CONFIG_FILE} would not parse after the change: {}",
                    e.message
                ),
                format!(
                    "set `[mode]` `name = \"{}\"` in {CONFIG_FILE} by hand, in a PR",
                    next.as_str()
                ),
            )
        })?;
    }
    if need_pin && s.bosun_channel {
        return Err(BosunError::conflict(
            format!(
                "bosun {} is pinned but a lighter mode needs {MODE_MIN} or newer, and this repo takes bosun from the channel",
                s.version.as_deref().unwrap_or("(no version)")
            ),
            "run `splice update --channel bosun` (or merge the splice bot's PR), then rerun `bosun migrate`",
        ));
    }
    let mut planned: Vec<String> = Vec::new();
    if need_pin {
        planned.push(VERSION_FILE.into());
    }
    if need_registry.is_some() {
        planned.push(REGISTRY_FILE.into());
    }
    if new_toml.is_some() {
        planned.push(CONFIG_FILE.into());
    }
    let mut data = json!({
        "from": from.as_str(), "to": to.as_str(), "step": next.as_str(), "direct": a.direct,
        "lighter": lighter, "dry_run": a.dry_run, "changed": !planned.is_empty(),
        "reused": false, "survey": s.json(), "files": planned, "pr": null, "branch": null,
        "owner": {"approval": approval(lighter, None), "steps": owner_steps(next)},
        "then": then(None, next, to),
    });
    if planned.is_empty() {
        data["then"] = json!([
            "nothing left to change here: `bosun migrate --finish` (join and amend, both idempotent)"
        ]);
        return Ok(Outcome::data(data));
    }
    if a.dry_run {
        return Ok(Outcome::data(data));
    }
    let repo = app.ctx.repo.to_string();
    if let Some(p) = find_open(app.gh.as_ref(), &repo)? {
        if branch_step(&p.head_ref) != Some(next.as_str()) {
            return Err(BosunError::conflict(
                format!(
                    "PR #{} ({}) is an open migration to another step than {}",
                    p.number,
                    p.head_ref,
                    next.as_str()
                ),
                format!("merge or close #{} first, then rerun", p.number),
            )
            .with_data(json!({"pr": {"number": p.number, "head_sha": p.head_sha}})));
        }
        data["reused"] = json!(true);
        data["changed"] = json!(false);
        data["files"] = json!([]);
        data["branch"] = json!(p.head_ref);
        data["pr"] = json!({"number": p.number, "head_sha": p.head_sha});
        data["owner"]["approval"] = json!(approval(lighter, Some((p.number, &p.head_sha))));
        data["then"] = json!(then(Some(p.number), next, to));
        return Ok(Outcome::data(data));
    }
    // Before anything is written.
    let base = default_branch(app.gh.as_ref())?;
    let on = git(&root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if on != base {
        return Err(BosunError::conflict(
            format!("on {on}, not the default branch {base}"),
            format!("run `git switch {base}` (up to date), then rerun"),
        ));
    }
    if !git(&root, &["status", "--porcelain", "--untracked-files=all"])?.is_empty() {
        return Err(BosunError::conflict(
            "the working tree has changes",
            "commit or stash them, then rerun",
        ));
    }
    git_net(&root, &["fetch", "-q", "origin", &base])?;
    let (head, remote) = (
        git(&root, &["rev-parse", "HEAD"])?,
        git(&root, &["rev-parse", "FETCH_HEAD"])?,
    );
    if head != remote {
        return Err(BosunError::conflict(
            format!("{base} is not at origin/{base} (HEAD {head}, origin {remote})"),
            "run `git pull --ff-only`, or push or drop the local commits, then rerun",
        ));
    }
    let branch = free_branch(&root, next)?;
    // From here on every failure restores the tree and the branch.
    let mut wrote = Wrote::default();
    let mut files: Vec<String> = Vec::new();
    let made = (|| -> Result<Pull, BosunError> {
        if need_pin {
            pin_bosun(&root, to, &mut files)?;
        }
        let write = |rel: &str, text: &str| {
            std::fs::write(root.join(rel), text).map_err(|e| {
                BosunError::error(
                    format!("cannot write {rel}: {e}"),
                    "check the file permissions, then rerun",
                )
            })
        };
        if let Some(n) = need_registry {
            write(REGISTRY_FILE, &format!("{n}\n"))?;
            files.push(REGISTRY_FILE.to_string());
        }
        if let Some(t) = &new_toml {
            write(CONFIG_FILE, t)?;
            files.push(CONFIG_FILE.to_string());
        }
        let title = format!("chore: bosun mode {} -> {}", from.as_str(), next.as_str());
        let text = body(&BodyIn {
            from,
            to,
            step: next,
            direct: a.direct,
            version: s.version.as_deref(),
            binary: env!("CARGO_PKG_VERSION"),
            registry: format!(
                "pinned {}, live {}",
                s.pinned.map_or("none".into(), |n| format!("#{n}")),
                s.live.first().map_or("none".into(), |n| format!("#{n}"))
            ),
            epics_unmarked: &s.epics_unmarked,
            hull: if s.hull_lock {
                format!(
                    "yes (version {})",
                    s.hull_version.as_deref().unwrap_or("unknown")
                )
            } else {
                "no".into()
            },
            baseline: s.baseline.as_ref().map(compact).map_err(Clone::clone),
            files: &files,
        });
        publish(
            &app, &root, &files, &base, &branch, &title, &text, &mut wrote,
        )
    })();
    let pull = match made {
        Ok(p) => p,
        Err(mut e) => {
            let drop_branch = (wrote.branch && !wrote.pushed).then_some(branch.as_str());
            let back = rollback(&root, &base, drop_branch, &files);
            if wrote.pushed {
                // The branch is on origin: the PR is the only step left.
                e.fix = format!(
                    "{branch} is pushed: open its PR by hand with `gh pr create --draft --head {branch} --base {base} --title \"chore: bosun mode {} -> {}\"`",
                    from.as_str(),
                    next.as_str()
                );
            } else if !back {
                e.fix = format!(
                    "could not restore the tree: run `git restore --staged --worktree .`, `git switch -f {base}` and `git branch -D {branch}`, then rerun"
                );
            } else if wrote.branch {
                e.fix = format!("rolled back to {base}; {}", e.fix);
            }
            return Err(e);
        }
    };
    data["files"] = json!(files);
    data["branch"] = json!(branch);
    data["pr"] = json!({"number": pull.number, "head_sha": pull.head_sha});
    data["owner"]["approval"] = json!(approval(lighter, Some((pull.number, &pull.head_sha))));
    data["then"] = json!(then(Some(pull.number), next, to));
    Ok(Outcome::data(data))
}

/// How far a run got, for the rollback.
#[derive(Default)]
struct Wrote {
    branch: bool,
    pushed: bool,
}

/// Put the tree back: return to `base`, drop `branch`, then undo exactly `files` (what bosun
/// wrote; the tree was clean before): a file in HEAD is restored, any other was created by bosun
/// and is deleted. Nothing else is touched. Whether it all worked.
fn rollback(root: &Path, base: &str, branch: Option<&str>, files: &[String]) -> bool {
    let mut ok = true;
    let on = git(root, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();
    if on != base {
        ok &= git(root, &["switch", "-q", "-f", base]).is_ok();
    }
    if let Some(b) = branch {
        ok &= git(root, &["branch", "-q", "-D", b]).is_ok();
    }
    for f in files {
        if git(root, &["cat-file", "-e", &format!("HEAD:{f}")]).is_ok() {
            ok &= git(
                root,
                &[
                    "restore",
                    "--source=HEAD",
                    "--staged",
                    "--worktree",
                    "--",
                    f,
                ],
            )
            .is_ok();
        } else {
            let _ = git(root, &["rm", "-q", "--cached", "--ignore-unmatch", "--", f]);
            let p = root.join(f);
            if p.exists() {
                ok &= std::fs::remove_file(p).is_ok();
            }
        }
    }
    ok
}

/// `bosun pin --latest --tool bosun`: the files it changed, plus `.bosun/version` when the repo
/// had none. A failure is a conflict that names the pin step and carries the child's own fix; a
/// release still below [`MODE_MIN`] gets the owner's release fix. The caller restores the tree.
fn pin_bosun(root: &Path, to: ModeName, written: &mut Vec<String>) -> Result<(), BosunError> {
    let pin = |why: String, fix: String| {
        BosunError::conflict(
            format!("the pin step (`bosun pin --latest --tool bosun`) failed: {why}"),
            fix,
        )
    };
    let (code, v) = run_self(
        root,
        &["pin", "--latest", "--tool", "bosun"],
        Duration::from_secs(300),
    )
    .map_err(|why| {
        pin(
            why,
            "run `bosun pin --latest --tool bosun` by hand to see why, then rerun `bosun migrate`"
                .into(),
        )
    })?;
    if code != 0 {
        let mut e = pin(error_text(&v), error_fix(&v));
        if let Some(x) = crate::output::Exit::ALL
            .into_iter()
            .find(|x| i32::from(x.code()) == code && *x != crate::output::Exit::Ok)
        {
            e.exit = x;
        }
        return Err(e);
    }
    written.extend(
        v["data"]["files"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|f| f.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    );
    // The version now pinned: pin's own report, else the script's line (already at the latest).
    let pinned = v["data"]["changed"]
        .as_array()
        .and_then(|c| c.iter().find(|c| c["tool"] == "bosun"))
        .and_then(|c| c["to"].as_str().map(str::to_string))
        .or_else(|| {
            let script = read(root, "docs/setup-script.sh")?;
            get_line(&script, &keys("bosun")[0]).map(str::to_string)
        });
    if let Some(ver) = &pinned
        && read(root, VERSION_FILE).as_deref().map(str::trim) != Some(ver.as_str())
    {
        std::fs::write(root.join(VERSION_FILE), format!("{ver}\n")).map_err(|e| {
            BosunError::error(
                format!("cannot write {VERSION_FILE}: {e}"),
                "check the file permissions, then rerun",
            )
        })?;
        if !written.iter().any(|f| f == VERSION_FILE) {
            written.push(VERSION_FILE.to_string());
        }
    }
    let now = read(root, VERSION_FILE).map(|v| v.trim().to_string());
    let still_below = match pinned.as_deref().or(now.as_deref()) {
        Some(v) => crate::core::pin::parse_semver(v).is_none() || semver_lt(v, MODE_MIN),
        None => true,
    };
    if still_below {
        return Err(pin(
            format!(
                "npm's newest bosun is {}, older than {MODE_MIN}",
                pinned.as_deref().unwrap_or("not recorded anywhere")
            ),
            format!(
                "the owner publishes a bosun release >= {MODE_MIN} (docs/releasing.md); then rerun `bosun migrate --to {}`",
                to.as_str()
            ),
        ));
    }
    Ok(())
}

/// Branch, commit, push (never forced), draft PR. `wrote` says how far it got; the caller rolls
/// back.
#[allow(clippy::too_many_arguments)]
fn publish(
    app: &App,
    root: &Path,
    files: &[String],
    base: &str,
    branch: &str,
    title: &str,
    body: &str,
    wrote: &mut Wrote,
) -> Result<Pull, BosunError> {
    git(root, &["switch", "-q", "-c", branch])?;
    wrote.branch = true;
    let mut add = vec!["add", "--"];
    add.extend(files.iter().map(String::as_str));
    git(root, &add)?;
    git(root, &["commit", "-q", "-m", title])?;
    let spec = format!("refs/heads/{branch}:refs/heads/{branch}");
    git_net(root, &["push", "-q", "-u", "origin", &spec])?;
    wrote.pushed = true;
    app.gh.create_pull(title, branch, base, body, true)
}

/// `--finish`: join, then amend each plan (both idempotent).
/// A plan path from an issue body or `--plan`: repo-relative, no `..`, a `.md` file that,
/// resolved (symlinks too), stays inside the repo.
fn plan_path_ok(root: &Path, p: &str) -> bool {
    use std::path::Component;
    let path = Path::new(p);
    if p.is_empty()
        || path.extension().and_then(|e| e.to_str()) != Some("md")
        || !path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return false;
    }
    // The nearest existing ancestor, resolved, must stay inside the repo (a missing file is
    // then plan amend's own error).
    let Ok(r) = root.canonicalize() else {
        return false;
    };
    let mut at = root.join(path);
    loop {
        if let Ok(f) = at.canonicalize() {
            return f.starts_with(&r);
        }
        if !at.pop() {
            return false;
        }
    }
}

fn finish(dir: Option<&Path>, app: &App, a: &MigrateArgs) -> CmdResult {
    let repo = app.ctx.repo.to_string();
    if let Some(p) = find_open(app.gh.as_ref(), &repo)? {
        return Err(BosunError::conflict(
            format!("PR #{} not merged yet ({})", p.number, p.head_ref),
            "wait for the merge, then rerun `bosun migrate --finish`",
        )
        .with_data(json!({"pr": {"number": p.number, "head_sha": p.head_sha}})));
    }
    super::join::run(
        dir,
        &JoinArgs {
            origin: None,
            area: None,
            parent: None,
            depth: None,
            slack_channel: None,
            slack_thread: None,
        },
    )?;
    let mut notes: Vec<String> = Vec::new();
    let (unmarked, found) = match epics(app.gh.as_ref()) {
        Ok(x) => x,
        Err(e) => {
            notes.push(format!("cannot read the epics: {}", e.message));
            (Vec::new(), Vec::new())
        }
    };
    let root = &app.ctx.dir;
    if let Some(bad) = a.plan.iter().find(|p| !plan_path_ok(root, p)) {
        return Err(BosunError::usage(
            format!("--plan {bad} is not a plan path inside the repo"),
            "name a repo-relative `.md` plan file with no `..`, for example plans/bosun.md",
        ));
    }
    let mut plans = a.plan.clone();
    for p in found {
        if !plan_path_ok(root, &p) {
            notes.push(format!(
                "an epic marker names {p}, which is not a plan path inside the repo: not amended; fix the epic body or pass `--plan <file>`"
            ));
        } else if !plans.contains(&p) {
            plans.push(p);
        }
    }
    // The merged config decides, not this checkout (which may be behind).
    let base = default_branch(app.gh.as_ref())?;
    let merged = match app.gh.file_at(CONFIG_FILE, &base)? {
        Some(t) => Config::parse(&t, &format!("{CONFIG_FILE} on {base}"))?,
        None => Config::default(),
    };
    let mode = merged.mode_effective().mode;
    let mut amended: Vec<Value> = Vec::new();
    let mut failed: Option<BosunError> = None;
    if super::budget::knob_from_default(app.gh.as_ref())? == BudgetFeature::Off {
        if !plans.is_empty() {
            notes.push("budget_feature is off: the plans are not amended".into());
        }
    } else {
        for file in &plans {
            let r = super::plan::run(
                dir,
                &PlanCmd::Amend(PlanAmendArgs {
                    file: file.clone(),
                    dry_run: false,
                }),
            );
            match r {
                Ok(_) => {
                    amended.push(json!({"plan": file, "ok": true, "exit_code": 0, "error": null}))
                }
                Err(e) => {
                    amended.push(json!({
                        "plan": file, "ok": false, "exit_code": e.exit.code(),
                        "error": {"kind": e.kind.unwrap_or(e.exit.name()), "message": e.message, "fix": e.fix},
                    }));
                    failed.get_or_insert(BosunError {
                        message: format!("plan amend {file} failed: {}", e.message),
                        ..e
                    });
                }
            }
        }
        if plans.is_empty() && !unmarked.is_empty() {
            for e in &unmarked {
                notes.push(format!(
                    "epic #{e} has no plan marker and no plan is named: rerun `bosun migrate --finish --plan <file>`"
                ));
            }
        }
    }
    let data = json!({
        "joined": true, "amended": amended, "epics_unmarked": unmarked,
        "mode": mode.as_str(), "notes": notes,
    });
    match failed {
        Some(e) => Err(e.with_data(data)),
        None => Ok(Outcome::data(data)),
    }
}
