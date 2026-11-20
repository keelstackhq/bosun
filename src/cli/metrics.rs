//! `bosun metrics`: red-CI rate, brake, cost, lead time, owner touches, blocks per rule (spec §18).
//! Read-only. Sources: `ci-ok` check runs of the Actions app (PR heads and merge commits of
//! main), merged revert PRs by a writer, PR and task comments. Main-red issues are never read.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::json;

use super::MetricsArgs;
use super::risk_check::{enrolled_keys, owner_logins};
use crate::app::{App, Result, default_branch, is_writer, spend_notes, task_meta, trusted_notes};
use crate::core::approve::Stat;
use crate::core::approve::{
    ApproverKey, Form, Role, signature_lines, signed_message, signed_message_v2, verify_ed25519,
};
use crate::core::budget_report::{facts, merge_spend};
use crate::core::cost::{revert_target, round_usd};
use crate::core::meta::closing_issues;
use crate::core::metrics::{
    CiRun, HeadCi, RuleBlocks, ci_head, owner_rules, p50, proposals, revert_of, waiting_rules,
};
use crate::core::mode::{BrakeWindow, brake};
use crate::core::records::{Record, parse_record};
use crate::core::risk::Rules;
use crate::ctx::RepoId;
use crate::github::b64;
use crate::github::gate::gate_history;
use crate::github::{Github, Pull};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::time::{Timestamp, parse_rfc3339};

const DAY: i64 = 86_400;
const BRAKE_DAYS: u32 = 7;

/// One own-branch PR with the `ci-ok` facts of its heads and, once merged, of its merge commit.
struct Facts {
    pull: Pull,
    heads: Vec<HeadCi>,
    main_red: bool,
    /// False when the PR changes `.github/**`: its own `on: pull_request` workflows post its
    /// `ci-ok`, so its heads never count for the (advisory) red-CI rate.
    rate: bool,
}

fn ts(t: &Option<String>) -> Option<Timestamp> {
    t.as_deref().and_then(parse_rfc3339)
}

fn ci(gh: &dyn Github, sha: &str, excuse: bool) -> Result<Option<HeadCi>> {
    let runs: Vec<CiRun> = gh
        .check_runs_all(sha)?
        .into_iter()
        .map(|r| CiRun {
            completed: r.status == "completed",
            at: r.completed_at.as_deref().and_then(parse_rfc3339),
            name: r.name,
            app: r.app,
            conclusion: r.conclusion,
        })
        .collect();
    Ok(ci_head(&runs, excuse))
}

/// PRs read per run unless `--max-prs` says otherwise.
const MAX_PRS: usize = 100;

/// A wall-clock limit (`BOSUN_METRICS_TIMEOUT_SECS`, default 90, fractions allowed, 0 = none), so
/// `metrics` ends with an error line instead of hanging behind a slow API. The HTTP budget
/// (spec §1) caps the calls; this caps the whole walk, local work included.
struct Clock {
    start: std::time::Instant,
    limit: Option<f64>,
    quiet: bool,
}

impl Clock {
    fn new() -> Result<Self> {
        let limit = match std::env::var("BOSUN_METRICS_TIMEOUT_SECS") {
            Err(_) => Some(90.0),
            Ok(v) => match v.trim().parse::<f64>() {
                Ok(s) if s.is_finite() && s >= 0.0 => (s > 0.0).then_some(s),
                _ => {
                    return Err(BosunError::usage(
                        format!(
                            "BOSUN_METRICS_TIMEOUT_SECS={:?} is not a number of seconds (0 or more)",
                            v.trim()
                        ),
                        "set BOSUN_METRICS_TIMEOUT_SECS to seconds (default 90; 0 = no limit), or unset it",
                    ));
                }
            },
        };
        Ok(Clock {
            start: std::time::Instant::now(),
            limit,
            quiet: false,
        })
    }

    /// No limit and no progress lines: for the brake that `heal` and `wave tick` read.
    fn silent() -> Self {
        Clock {
            start: std::time::Instant::now(),
            limit: None,
            quiet: true,
        }
    }

    fn check(&self, done: usize, total: usize, what: &str) -> Result<()> {
        let spent = self.start.elapsed().as_secs_f64();
        match self.limit {
            Some(l) if spent > l => {
                let message =
                    format!("metrics timed out after {spent:.0}s ({done} of {total} {what} read)");
                eprintln!("{message}");
                Err(BosunError::error(
                    message,
                    "narrow it (`bosun metrics --days 1`, or `--max-prs 20`), or raise BOSUN_METRICS_TIMEOUT_SECS (0 = no limit)",
                ))
            }
            _ => Ok(()),
        }
    }
}

/// The PRs read, and how many newer-first PRs past the cap were left out.
struct Collected {
    facts: Vec<Facts>,
    skipped: usize,
}

fn collect(app: &App, since: Timestamp, max_prs: usize, clock: &Clock) -> Result<Collected> {
    let gh = app.gh.as_ref();
    let repo = app.ctx.repo.to_string();
    let mut out = Vec::new();
    let mut pulls = gh.list_pulls_updated_since(&since.to_string())?;
    // Newest update first: the cap keeps the PRs that matter most to the window.
    let skipped = pulls.len().saturating_sub(max_prs);
    pulls.truncate(max_prs);
    let total = pulls.len();
    for (i, pull) in pulls.into_iter().enumerate() {
        clock.check(i, total, "PRs")?;
        if !clock.quiet {
            eprintln!("metrics: PR {}/{total} (#{})", i + 1, pull.number);
        }
        // A fork's heads are not ours to count; its merge into main is.
        let (mut heads, mut rate) = (Vec::new(), false);
        if pull.head_repo.eq_ignore_ascii_case(&repo) {
            rate = !gh.pull_files(pull.number)?.iter().any(|f| {
                std::iter::once(&f.filename)
                    .chain(&f.previous_filename)
                    .any(|n| n.starts_with(".github/"))
            });
            let shas = gh.pull_commits(pull.number)?;
            for (i, sha) in shas.iter().enumerate() {
                // Superseded heads (run cancelled by the push) and a draft's head (`ci` not run).
                heads.extend(ci(gh, sha, i + 1 < shas.len() || pull.draft)?);
            }
        } else if pull.merged_at.is_none() {
            continue;
        }
        let main_red = match (&pull.merged_at, &pull.merge_commit_sha) {
            // Pushes to main never cancel each other: any red `ci-ok` there is red.
            (Some(_), Some(sha)) => ci(gh, sha, false)?.is_some_and(|h| h.failing),
            _ => false,
        };
        out.push(Facts {
            pull,
            heads,
            main_red,
            rate,
        });
    }
    Ok(Collected {
        facts: out,
        skipped,
    })
}

/// The brake inputs over the last 7 days. Later commands call this, then `core::mode::brake`.
pub fn window(app: &App) -> Result<BrakeWindow> {
    let since = app.ctx.now.minus_secs(i64::from(BRAKE_DAYS) * DAY);
    let c = collect(app, since, usize::MAX, &Clock::silent())?;
    brake_window(app, &c.facts, since)
}

fn brake_window(app: &App, facts: &[Facts], since: Timestamp) -> Result<BrakeWindow> {
    let mut w = BrakeWindow {
        days: BRAKE_DAYS,
        main_red: 0,
        reverts: 0,
        failing_heads: 0,
        pushed_heads: 0,
    };
    for f in facts {
        let merged = ts(&f.pull.merged_at).filter(|t| *t >= since);
        w.main_red += u32::from(merged.is_some() && f.main_red);
        if merged.is_some()
            // The branch name cannot be edited after the merge; the body can.
            && (revert_target(&f.pull.head_ref).is_some() || revert_of(&f.pull.body).is_some())
            && is_writer(app.gh.as_ref(), &f.pull.user)?
        {
            w.reverts += 1;
        }
        (w.pushed_heads, w.failing_heads) = rate_add(f, since, (w.pushed_heads, w.failing_heads));
    }
    Ok(w)
}

/// Adds `f`'s heads in the window to `(pushed, failing)`.
fn rate_add(f: &Facts, since: Timestamp, (p, r): (u32, u32)) -> (u32, u32) {
    let heads = f
        .heads
        .iter()
        .filter(|h| f.rate && h.at.is_none_or(|t| t >= since));
    heads.fold((p, r), |(p, r), h| (p + 1, r + u32::from(h.failing)))
}

/// A comment signed with an enrolled owner-role key in the plain or after-review form; the
/// orchestrator's bootstrap and autonomous forms and delegate keys never count.
fn owner_signed(body: &str, keys: &[ApproverKey], repo: &RepoId, pr: u64) -> bool {
    let first = body.lines().next().unwrap_or_default().trim();
    let head = (first.strip_prefix("/bosun approve ")).and_then(|r| r.get(..40));
    let head = head.unwrap_or_default();
    let (o, r) = (&repo.owner, &repo.name);
    signature_lines(body, head)
        .into_iter()
        .any(|(form, id, sig)| {
            let msg = match form {
                Form::V1 => signed_message(Role::Owner, o, r, pr, head),
                Form::V2(v) => signed_message_v2(o, r, pr, head, v),
                _ => return false,
            };
            keys.iter().any(|k| {
                k.id == id
                    && k.role == Role::Owner
                    && (b64::decode(&k.ed25519).zip(b64::decode(sig)))
                        .is_some_and(|(k, s)| verify_ed25519(&k, msg.as_bytes(), &s))
            })
        })
}

type Blocks = BTreeMap<String, (Vec<u64>, u32, u32)>;

/// `(caught, passed)` of a PR closed or merged since `start`: caught = closed unmerged or
/// re-pushed, passed = merged on the first head. `None` for the rest (open, older).
fn outcome(f: &Facts, start: Timestamp) -> Option<(bool, bool)> {
    let in_days = |t: &Option<String>| ts(t).is_some_and(|t| t >= start);
    let p = &f.pull;
    match (p.merged_at.is_some(), p.state == "closed") {
        (false, true) if in_days(&p.closed_at) => Some((true, false)),
        (true, _) if in_days(&p.merged_at) => Some((f.heads.len() > 1, f.heads.len() == 1)),
        _ => None,
    }
}

/// Blocks per rule since `start` from the Risk notes: the blocked PRs, caught and passed.
fn rule_blocks(all: &[Facts], start: Timestamp) -> Blocks {
    let mut blocks = Blocks::new();
    for f in all {
        let Some((caught, passed)) = outcome(f, start) else {
            continue;
        };
        // A rule listed twice in one note blocks that PR once.
        for rule in owner_rules(&f.pull.body)
            .into_iter()
            .collect::<BTreeSet<_>>()
        {
            let e = blocks.entry(rule).or_default();
            e.0.push(f.pull.number);
            (e.1, e.2) = (e.1 + u32::from(caught), e.2 + u32::from(passed));
        }
    }
    blocks
}

/// As [`rule_blocks`], but the rule ids come from the trusted risk-check posts on the PR's
/// last head (the body is agent-editable), and only passed PRs are listed.
fn trusted_blocks(gh: &dyn Github, all: &[Facts], start: Timestamp) -> Result<Vec<RuleBlocks>> {
    let mut by: BTreeMap<String, RuleBlocks> = BTreeMap::new();
    for f in all {
        let Some((caught, passed)) = outcome(f, start) else {
            continue;
        };
        let history: Vec<Stat> = gate_history(gh, &f.pull.head_sha)?
            .into_iter()
            .map(|s| Stat {
                context: s.context,
                state: s.state,
                description: s.description,
                creator: s.creator,
            })
            .collect();
        for rule in waiting_rules(&history) {
            let e = by.entry(rule.clone()).or_insert(RuleBlocks {
                rule,
                passed: Vec::new(),
                caught: 0,
            });
            e.passed.extend(passed.then_some(f.pull.number));
            e.caught += u32::from(caught);
        }
    }
    Ok(by.into_values().collect())
}

/// `--propose`: the rules a curator may propose to loosen, each with a draft PR title and body.
/// Read-only; the curator Routine opens the PR (R-weaken, so the owner decides).
fn propose(app: &App, days: u32) -> CmdResult {
    let start = app.ctx.now.minus_secs(i64::from(days) * DAY);
    let c = collect(app, start, MAX_PRS, &Clock::new()?)?;
    let skipped = c.skipped;
    let all = trusted_blocks(app.gh.as_ref(), &c.facts, start)?;
    // Only rules the base `.bosun/risk.toml` defines can be loosened there (not built-ins
    // such as R-unseen, not ids of a stale or forged note).
    let gh = app.gh.as_ref();
    let rules = match gh.file_at(".bosun/risk.toml", &default_branch(gh)?)? {
        Some(t) => Rules::parse(&t).map_err(|e| {
            BosunError::error(
                format!(".bosun/risk.toml on the default branch: {e}"),
                "fix .bosun/risk.toml on the default branch",
            )
        })?,
        None => Rules::defaults(),
    };
    let defined: Vec<String> = rules.rules.into_iter().map(|r| r.id).collect();
    let knob = app.ctx.config.mode_effective().knobs.curator_loosen.value;
    let found = proposals(&all, &defined, days).into_iter().map(|p| {
        json!({"rule": p.rule, "blocks": p.blocks, "prs": p.prs, "title": p.title, "body": p.body})
    });
    let mut data = json!({
        "days": days, "since": start.to_string(), "curator_loosen": knob.as_str(),
        "proposals": found.collect::<Vec<_>>(),
    });
    if skipped > 0 {
        data["capped"] = json!({"max_prs": MAX_PRS, "skipped": skipped});
    }
    Ok(Outcome::data(data))
}

pub fn run(dir: Option<&Path>, args: &MetricsArgs) -> CmdResult {
    if args.max_prs == 0 {
        return Err(BosunError::usage(
            "--max-prs must be at least 1",
            "pass --max-prs 100",
        ));
    }
    let days = args.days;
    if days == 0 {
        return Err(BosunError::usage(
            "--days must be at least 1",
            "pass --days 30",
        ));
    }
    let app = App::load(dir)?;
    if args.propose {
        return propose(&app, days);
    }
    let (gh, now) = (app.gh.as_ref(), app.ctx.now);
    let start = now.minus_secs(i64::from(days) * DAY);
    let b7 = now.minus_secs(i64::from(BRAKE_DAYS) * DAY);
    let clock = Clock::new()?;
    let Collected {
        facts: all,
        skipped,
    } = collect(&app, start.min(b7), args.max_prs, &clock)?;
    let w = brake_window(&app, &all, b7)?;
    let eff = app.ctx.config.mode_effective();
    let state = brake(eff.mode, &w);
    let in_days = |t: &Option<String>| ts(t).is_some_and(|t| t >= start);

    // Cost per merged PR, lead time per epic (first claim to last merge).
    let (mut merged, mut usd) = (0, 0.0);
    let mut paid: BTreeSet<u64> = BTreeSet::new();
    let mut epics: BTreeMap<u64, (Option<Timestamp>, Timestamp)> = BTreeMap::new();
    let mut threads: BTreeSet<u64> = BTreeSet::new();
    let mut n_seen = 0;
    for f in all.iter().filter(|f| {
        ts(&f.pull.created_at).is_some_and(|t| t >= start)
            || in_days(&f.pull.merged_at)
            || in_days(&f.pull.closed_at)
    }) {
        threads.insert(f.pull.number);
        n_seen += 1;
        clock.check(n_seen, all.len(), "PRs")?;
        let task = closing_issues(&f.pull.body).first().copied();
        threads.extend(task);
        let Some(at) = ts(&f.pull.merged_at).filter(|t| *t >= start) else {
            continue;
        };
        merged += 1;
        // A task's spend counts once, however many of its PRs merged.
        let task_spend = match task {
            Some(t) if paid.insert(t) => spend_notes(gh, t)?,
            _ => Vec::new(),
        };
        usd += facts(&merge_spend(task_spend, spend_notes(gh, f.pull.number)?)).2;
        let Some(task) = task else { continue };
        let epic = gh
            .get_issue(task)?
            .and_then(|i| task_meta(&i))
            .and_then(|m| m.epic);
        if let Some(e) = epic {
            let claim = trusted_notes(gh, task)?
                .iter()
                .filter(|n| matches!(parse_record(&n.body), Some(Record::Lease { .. })))
                .map(|n| n.created_at)
                .min();
            let s = epics.entry(e).or_insert((None, at));
            s.0 = s.0.into_iter().chain(claim).min();
            s.1 = s.1.max(at);
        }
    }
    let mut hours: Vec<f64> = epics
        .values()
        .filter_map(|(c, m)| Some((m.0 - c.as_ref()?.0) as f64 / 3600.0))
        .collect();
    let round2 = |x: f64| (x * 100.0).round() / 100.0;

    // Owner touches: an owner's own approve command or answer, or an owner-key signature.
    let owners = owner_logins(gh, &default_branch(gh)?)?;
    let keys = enrolled_keys(gh).unwrap_or_default();
    let mut touches = 0;
    for (i, n) in threads.iter().enumerate() {
        clock.check(i, threads.len(), "threads")?;
        for c in gh.list_comments(*n)? {
            // A body with a signature line is judged by its signature alone.
            let signed = c.body.lines().any(|l| l.trim().starts_with("bosun-sig "));
            let by_owner = owners.iter().any(|o| o.eq_ignore_ascii_case(&c.user))
                && !signed
                && (c.body.starts_with("/bosun approve")
                    || matches!(parse_record(&c.body), Some(Record::Answer { .. })));
            let counts = in_days(&Some(c.created_at.clone()))
                && (by_owner || owner_signed(&c.body, &keys, &app.ctx.repo, *n));
            touches += u32::from(counts);
        }
    }

    let blocks = rule_blocks(&all, start);
    let (pushed, failing) = all.iter().fold((0, 0), |a, f| rate_add(f, start, a));
    let rate =
        (pushed > 0).then(|| (f64::from(failing) * 1000.0 / f64::from(pushed)).round() / 10.0);
    let mut data = json!({
        "days": days, "since": start.to_string(), "mode": eff.mode.as_str(),
        "brake": {"knob": eff.knobs.brake.value.as_str(), "on": state.on, "reasons": state.reasons,
            "capped": skipped > 0,
            "window": {"days": w.days, "main_red": w.main_red, "reverts": w.reverts,
                "failing_heads": w.failing_heads, "pushed_heads": w.pushed_heads}},
        "red_ci": {"failing_heads": failing, "pushed_heads": pushed, "rate_pct": rate},
        "merged": merged,
        "cost_per_merged_usd": (merged > 0).then(|| round_usd(usd / f64::from(merged))),
        "lead_time_p50_hours": p50(&mut hours).map(round2),
        "epics": hours.len(),
        "owner_touches": touches,
        "owner_touches_per_day": round2(f64::from(touches) / f64::from(days)),
        "blocks": blocks.iter().map(|(r, (b, c, p))| json!({"rule": r, "blocks": b.len(), "caught": c, "passed": p})).collect::<Vec<_>>(),
    });
    if skipped > 0 {
        eprintln!(
            "metrics: {skipped} older PRs left out (--max-prs {})",
            args.max_prs
        );
        data["capped"] = json!({"max_prs": args.max_prs, "skipped": skipped});
    }
    Ok(Outcome::data(data))
}
