//! `bosun wait`: block until a PR, an issue or a child's result changes (spec §5, D14). One
//! call polls GitHub with backoff; it never uses send_later or send_message.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::WaitArgs;
use crate::app::{App, kill_switch, notes};
use crate::core::records::{Record, parse_record};
use crate::core::state::state_of;
use crate::github::{CheckRun, Github, IssueState};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::time::Timestamp;

/// Base delay between polls in milliseconds; `BOSUN_WAIT_BASE_MS` overrides it (tests).
const BASE_MS: u64 = 5000;
/// The delay stops doubling at 12 times the base (5 s becomes 60 s).
const CAP_FACTOR: u64 = 12;

enum Target {
    Pr(u64),
    Issue(u64),
    Child(String),
}

pub fn run(dir: Option<&Path>, args: &WaitArgs) -> CmdResult {
    let target = match (args.pr, args.issue, &args.child) {
        (Some(n), None, None) => Target::Pr(n),
        (None, Some(n), None) => Target::Issue(n),
        (None, None, Some(s)) => Target::Child(s.clone()),
        _ => {
            return Err(BosunError::usage(
                "wait needs exactly one of --pr, --issue, --child",
                "run `bosun wait --pr N`, `--issue N` or `--child SESSION`",
            ));
        }
    };
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    kill_switch(gh)?;
    let marker_dir = dir
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok());
    let _marker = marker_dir.map(|d| crate::hooks::stop::WaitMarker::start(&d, &app.ctx.session));
    let base = std::env::var("BOSUN_WAIT_BASE_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(BASE_MS);
    let beat_every = std::env::var("BOSUN_WAIT_BEAT_MS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map_or(Duration::from_secs(BEAT_SECS), Duration::from_millis);
    let (mut delay, cap) = (base, base * CAP_FACTOR);
    let timeout = if args.once { 0 } else { args.timeout };
    let deadline = Instant::now() + Duration::from_secs(timeout);
    if timeout > 0 {
        // Blocking is the point of a wait; the per-call timeouts still apply.
        gh.set_budget(None);
    }

    // Keep the caller alive while it waits (its leases and registry record), and reap what
    // expired, at the start and every BEAT_SECS
    // (`BOSUN_WAIT_BEAT_MS` overrides it, tests). `now` moves on with the wall clock.
    let start = Instant::now();
    let before = snapshot(gh, &target)?;
    let mut keepalive = Keepalive::default();
    keepalive.beat(gh, &app.ctx.session, app.ctx.now);
    let mut last_beat = start;

    let (mut polls, mut after, mut changed) = (1u64, before.clone(), false);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(delay).min(deadline - Instant::now()));
        if last_beat.elapsed() >= beat_every {
            let now = app.ctx.now.plus_secs(start.elapsed().as_secs() as i64);
            keepalive.beat(gh, &app.ctx.session, now);
            last_beat = Instant::now();
        }
        after = snapshot(gh, &target)?;
        polls += 1;
        if after != before {
            changed = true;
            break;
        }
        delay = (delay * 2).min(cap);
    }
    Ok(Outcome::data(json!({
        "changed": changed, "polls": polls, "before": before, "after": after,
        "heartbeat": keepalive.json(),
    })))
}

/// Seconds between two heartbeats of a wait: well inside the lease TTL and the 20-minute rule.
const BEAT_SECS: u64 = 600;

/// What the heartbeats of one wait did. A failed heartbeat never ends the wait: it is reported
/// in `error` (and on stderr), and the next one tries again.
#[derive(Default)]
struct Keepalive {
    beats: u64,
    leases: Vec<Value>,
    registry: bool,
    reaped: Vec<Value>,
    left: Vec<Value>,
    error: Option<String>,
}

impl Keepalive {
    fn beat(&mut self, gh: &dyn Github, session: &str, now: Timestamp) {
        // Halted (the kill switch opened while waiting): write nothing, keep polling.
        match crate::app::halt_reason(gh) {
            Ok(None) => {}
            Ok(Some((why, _))) => {
                self.error = Some(format!("halted ({why}); no heartbeat"));
                return;
            }
            Err(e) => {
                self.error = Some(e.message);
                return;
            }
        }
        // Renew first, so this session's own leases are live when the reap looks.
        let done = super::heartbeat::beat(gh, session, now).and_then(|mut v| {
            let (reaped, left) = super::reap::expire(gh, now)?;
            v["reaped"] = json!(reaped);
            v["left"] = json!(left);
            Ok(v)
        });
        match done {
            Ok(v) => {
                let list = |k: &str| v[k].as_array().cloned().unwrap_or_default();
                self.beats += 1;
                self.leases = list("leases");
                self.registry = v["registry"].as_bool().unwrap_or(false);
                self.reaped.extend(list("reaped"));
                self.left.extend(list("left"));
                self.error = None;
            }
            Err(e) => {
                eprintln!(
                    "bosun: warning: heartbeat while waiting failed: {}",
                    e.message
                );
                self.error = Some(e.message);
            }
        }
    }

    fn json(&self) -> Value {
        json!({"beats": self.beats, "leases": self.leases, "registry": self.registry,
            "reaped": self.reaped, "left": self.left, "error": self.error})
    }
}

/// The conclusion of the newest run (highest id) of each check name: a re-run adds a run, and
/// the older failed one must not make a green PR look red.
fn newest_checks(runs: Vec<CheckRun>) -> BTreeMap<String, Option<String>> {
    let mut newest: BTreeMap<String, CheckRun> = BTreeMap::new();
    for r in runs {
        if newest.get(&r.name).is_none_or(|o| r.id >= o.id) {
            newest.insert(r.name.clone(), r);
        }
    }
    newest.into_iter().map(|(k, r)| (k, r.conclusion)).collect()
}

fn snapshot(gh: &dyn Github, target: &Target) -> Result<Value, BosunError> {
    match target {
        Target::Pr(n) => {
            let pr = gh.get_pull(*n)?.ok_or_else(|| {
                BosunError::error(
                    format!("PR #{n} does not exist"),
                    "check the PR number; `bosun status` lists the open PRs",
                )
            })?;
            let statuses: BTreeMap<String, String> = gh
                .statuses(&pr.head_sha)?
                .into_iter()
                .map(|s| (s.context, s.state))
                .collect();
            let runs = newest_checks(gh.check_runs(&pr.head_sha)?);
            // Nothing has reported for this head yet: say so, not an empty map.
            let checks = if runs.is_empty() && statuses.is_empty() {
                json!("no checks yet")
            } else {
                json!(runs)
            };
            Ok(
                json!({"state": pr.state, "merged": pr.merged, "head_sha": pr.head_sha,
                "statuses": statuses, "checks": checks}),
            )
        }
        Target::Issue(n) => {
            let issue = gh.get_issue(*n)?.ok_or_else(|| {
                BosunError::error(
                    format!("issue #{n} does not exist"),
                    "check the issue number",
                )
            })?;
            Ok(json!({"state": state_of(&issue.labels).map(|s| s.name()),
                "open": issue.state == "open",
                "comments": gh.list_comments(*n)?.len()}))
        }
        Target::Child(session) => {
            let mut newest = Value::Null;
            for issue in gh.list_issues(&[], IssueState::Open)? {
                for note in notes(gh, issue.number)? {
                    if let Some(Record::Result {
                        session: s,
                        outcome,
                        pr,
                    }) = parse_record(&note.body)
                        && &s == session
                    {
                        newest = json!({"issue": issue.number, "comment_id": note.id,
                            "outcome": outcome, "pr": pr});
                    }
                }
            }
            Ok(json!({"result": newest}))
        }
    }
}
