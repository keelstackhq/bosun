//! `bosun approve`: lists the PRs that wait for an owner. The default approval (D-012): the
//! session asks the owner with a form in the Claude Code chat and the orchestrator posts the
//! signed comment (`--sign-key`). The fallback: the owner, on their own machine, posts the
//! comment `/bosun approve <head>` (spec section 8).
//! Every GitHub call uses `BOSUN_OWNER_TOKEN`, never `GITHUB_TOKEN`: the comment must come from
//! a plain user token, which agent sessions do not have. The owner names the head sha of each
//! PR (`<pr>@<sha>`, from `--list`), so a push after the listing is never approved unseen.

use std::path::Path;

use serde_json::{Value, json};

use super::ApproveArgs;
use crate::app::App;
use crate::app::{Result, default_ref, kill_switch, main_red};
use crate::cli::risk_check::{
    decisive_review, dismissed_review, enrolled_keys, owner_comment, owner_logins,
};
use crate::config::{CONFIG_FILE, Config};
use crate::core::approve::{
    APPROVERS_PATH, Form, Readiness, Role, Run, Stat, api_url_ok, approve_command, batch_message,
    bootstrap_line, in_cloud, parse_target, readiness, signed_comment, signed_message,
    signed_message_bootstrap, signed_message_v2,
};
use crate::core::records::{PrHead, Record};
use crate::ctx::Ctx;
use crate::gates::main_open::{is_mainfix, judge};
use crate::github::{Github, IssueState, LiveGithub, Pull, b64};
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;

const TOKEN_VAR: &str = "BOSUN_OWNER_TOKEN";
const MAIN_OPEN_WORKFLOW: &str = ".github/workflows/main-open.yml";

struct Ready {
    pr: u64,
    title: String,
    head_sha: String,
    base_sha: String,
    risk: Vec<String>,
    files: Vec<String>,
}

/// GitHub's compare API lists at most this many files and does not say when it cut the list.
const COMPARE_FILES_CAP: usize = 300;

/// Why a PR is not ready when an owner's newest decisive review on the head requests changes.
const WITHDRAWN: &str = "an owner's review requests changes on this head; that owner approves again with a GitHub review, or another owner approves with a GitHub review";

/// The same when that review was dismissed.
const WITHDRAWN_DISMISSED: &str = "an owner's request for changes on this head was dismissed; it needs a fresh plain owner comment (`bosun approve <pr>@<head>`, with the owner token), a signature bound to the dismissed review (`bosun approve <pr>@<head> --sign-key <pem>`), or a GitHub review from that owner, or a GitHub review from another owner";

/// Why an owner's approval of the head is withdrawn, if it is, and whether a fresh plain owner
/// comment ends it (only after a dismissal). A request for changes wins.
fn withdrawn(gh: &dyn Github, p: &Pull) -> Result<Option<(&'static str, bool)>> {
    let owners = owner_logins(gh, &p.base_sha)?;
    let reviews = gh.list_reviews(p.number)?;
    let states: Vec<_> = owners
        .iter()
        .map(|o| decisive_review(&reviews, o, &p.head_sha))
        .collect();
    let has = |s: &str| states.iter().any(|x| x.as_deref() == Some(s));
    Ok(if has("CHANGES_REQUESTED") {
        Some((WITHDRAWN, false))
    } else if has("DISMISSED") {
        Some((WITHDRAWN_DISMISSED, true))
    } else {
        None
    })
}

/// What to do instead in a Claude cloud session.
const CLOUD_FIX: &str = "ask the owner with a form in the Claude Code chat; on approve the orchestrator posts the signed comment (`bosun approve <pr>@<head> --sign-key <pem>`); fallback: the owner runs `bosun approve` on their own machine, or comments `/bosun approve <sha>` in the GitHub web or mobile app";

/// Shown with every list: an approval comment counts only when posted with a plain user token, as
/// this command does (an owner review counts too). The Claude GitHub tool posts through an app and
/// adds a footer, so its comments never count.
const LIST_HINT: &str = "ask the owner with a form in the Claude Code chat (repo, PR, head; approve or hold); on approve the orchestrator posts the signed comment (`bosun approve <pr>@<head> --sign-key <pem>`); fallback: the owner runs data.approve_command (bosun approve) on their own machine or approves in the GitHub UI; an unsigned /bosun approve comment posted through Claude's GitHub tool never counts; relay data.message and keep data.message_pairs: only the latest message is answerable";

struct Board {
    ready: Vec<Ready>,
    waiting: Vec<(u64, &'static str)>,
    /// Waiting PRs whose dismissed withdrawal a fresh `bosun approve` comment ends: (pr, head, base).
    dismissed: Vec<(u64, String, String)>,
}

/// Put `p` in `waiting` when an owner's approval of it is withdrawn.
fn hold(gh: &dyn Github, p: &Pull, board: &mut Board) -> Result<bool> {
    let Some((why, by_comment)) = withdrawn(gh, p)? else {
        return Ok(false);
    };
    board.waiting.push((p.number, why));
    if by_comment {
        let (head, base) = (p.head_sha.clone(), p.base_sha.clone());
        board.dismissed.push((p.number, head, base));
    }
    Ok(true)
}

/// The open PRs whose risk-check waits for an owner, split into ready and waiting, by number.
/// Statuses, check runs and files all describe the head sha read with the PR.
fn board(gh: &dyn Github) -> Result<Board> {
    let mut pulls = gh.list_pulls(None, IssueState::Open)?;
    pulls.sort_by_key(|p| p.number);
    let mut board = Board {
        ready: Vec::new(),
        waiting: Vec::new(),
        dismissed: Vec::new(),
    };
    for p in pulls {
        let statuses: Vec<Stat> = gh
            .status_history(&p.head_sha)?
            .into_iter()
            .map(|s| Stat {
                context: s.context,
                state: s.state,
                description: s.description,
                creator: s.creator,
            })
            .collect();
        let runs: Vec<Run> = gh
            .check_runs(&p.head_sha)?
            .into_iter()
            .map(|r| Run {
                id: r.id,
                name: r.name,
                conclusion: r.conclusion,
                at: r.completed_at.or(r.started_at).unwrap_or_default(),
                app: r.app,
            })
            .collect();
        match readiness(&statuses, &runs) {
            Readiness::NotWaiting => {}
            Readiness::Waiting(why) => board.waiting.push((p.number, why)),
            Readiness::Ready(_) if hold(gh, &p, &mut board)? => {}
            Readiness::Ready(risk) => board.ready.push(Ready {
                pr: p.number,
                title: p.title.clone(),
                files: gh
                    .compare_files(&p.base_sha, &p.head_sha)?
                    .into_iter()
                    .map(|f| f.filename)
                    .collect(),
                head_sha: p.head_sha,
                base_sha: p.base_sha,
                risk,
            }),
        }
    }
    Ok(board)
}

pub fn run(dir: Option<&Path>, args: &ApproveArgs) -> CmdResult {
    if !args.list && args.main_open.is_none() && args.prs.is_empty() {
        return Err(BosunError::usage(
            "approve needs PR@HEAD arguments, --list or --main-open PR",
            "run `bosun approve --list`, then the command in its data.approve_command",
        ));
    }
    if let Some(key) = &args.sign_key {
        return sign(dir, args, key);
    }
    let mut targets = Vec::new();
    for a in &args.prs {
        targets.push(parse_target(a).ok_or_else(|| {
            BosunError::usage(
                format!("{a:?} is not <pr>@<40-hex head sha>"),
                "run `bosun approve --list` and run the command in its data.approve_command",
            )
        })?);
    }
    let cloud = in_cloud(
        std::env::var("CCR_AGENT_PROXY_ENABLED").ok().as_deref(),
        std::env::var("CLAUDE_CODE_REMOTE_ENVIRONMENT_TYPE")
            .ok()
            .as_deref(),
    );
    if cloud && !args.list {
        return Err(BosunError::needs_human(
            "a Claude cloud session's agent proxy rewrites GitHub writes (app attribution and a footer), so an approval from here never counts",
            CLOUD_FIX,
        ));
    }
    let token = std::env::var(TOKEN_VAR)
        .ok()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| {
            BosunError::needs_human(
                format!("the owner's token is missing: {TOKEN_VAR} is not set"),
                format!(
                    "run bosun approve on your own machine (a local shell or a local Claude Code session) with {TOKEN_VAR} set; see docs/PROGRESS.md, Blocked 9. It never uses GITHUB_TOKEN"
                ),
            )
        })?;
    let token = token.trim().to_string();
    let api = std::env::var("BOSUN_GITHUB_API").unwrap_or_else(|_| "https://api.github.com".into());
    if !api_url_ok(&api) {
        return Err(BosunError::error(
            "BOSUN_GITHUB_API is not https://, so the owner token is not sent",
            "unset BOSUN_GITHUB_API (or set it to an https:// URL) in the owner's environment",
        ));
    }
    let ctx = Ctx::load(dir)?;
    let gh = LiveGithub::with_token(&token, &ctx.repo.owner, &ctx.repo.name);
    let now = ctx.now.to_string();
    redact(&token, exec(&gh, args, targets, &now, cloud))
}

fn exec(
    gh: &dyn Github,
    args: &ApproveArgs,
    mut targets: Vec<(u64, String)>,
    now: &str,
    cloud: bool,
) -> CmdResult {
    if args.list {
        let b = board(gh)?;
        let heads: Vec<(u64, String)> =
            b.ready.iter().map(|r| (r.pr, r.head_sha.clone())).collect();
        let items: Vec<(u64, String, String)> = b
            .ready
            .iter()
            .map(|r| (r.pr, r.head_sha.clone(), r.title.clone()))
            .collect();
        let (message_id, message, message_pairs) = match batch_message(&items) {
            Some(m) => (json!(m.id), json!(m.text), json!(m.pairs)),
            None => (Value::Null, Value::Null, Value::Null),
        };
        return Ok(Outcome::data(json!({
            "message_id": message_id,
            "message": message,
            "message_pairs": message_pairs,
            "ready": b.ready.iter().map(ready_json).collect::<Vec<_>>(),
            "waiting": b.waiting.iter().map(|(pr, why)| json!({"pr": pr, "why": why}))
                .collect::<Vec<_>>(),
            "approve_command": approve_command(&heads),
            "cloud": cloud,
            "hint": if cloud {
                format!("{LIST_HINT}; in a Claude cloud session an approval posted here never counts: {CLOUD_FIX}")
            } else {
                LIST_HINT.to_string()
            },
        })));
    }
    kill_switch(gh)?;
    if let Some(n) = args.main_open {
        let head = post_main_open(gh, n)?;
        write_record(gh, &[], std::slice::from_ref(&head), now)?;
        return Ok(Outcome::data(
            json!({"main_open": {"pr": n, "head_sha": head.head_sha}}),
        ));
    }

    let b = board(gh)?;
    targets.sort_unstable();
    targets.dedup();
    let mut not_ready = Vec::new();
    for (n, sha) in &targets {
        let why = match b.ready.iter().find(|r| r.pr == *n) {
            Some(r) if r.head_sha == *sha => continue,
            Some(r) => format!("head changed (now {})", r.head_sha),
            None => match b.dismissed.iter().find(|d| d.0 == *n) {
                Some(d) if d.1 == *sha => continue,
                Some(d) => format!("head changed (now {})", d.1),
                None => why_not(gh, &b, *n)?,
            },
        };
        not_ready.push(json!({"pr": n, "why": why}));
    }
    if !not_ready.is_empty() {
        return Err(BosunError::conflict(
            "some of the PRs are not ready at the head you named; nothing was posted",
            "run `bosun approve --list` again and run the command in its data.approve_command",
        )
        .with_data(json!({"not_ready": not_ready})));
    }
    let mut approved = Vec::new();
    let mut heads = Vec::new();
    let mut failed = None;
    for (pr, sha) in targets {
        let base = b
            .ready
            .iter()
            .find(|r| r.pr == pr)
            .map(|r| &r.base_sha)
            .or_else(|| b.dismissed.iter().find(|d| d.0 == pr).map(|d| &d.2))
            .expect("checked above");
        match approve_one(gh, pr, &sha, base) {
            Ok((id, repost)) => {
                let mut entry = json!({"pr": pr, "head_sha": sha, "comment_id": id});
                if repost {
                    entry["repost"] = json!(true);
                }
                approved.push(entry);
                heads.push(PrHead { pr, head_sha: sha });
            }
            Err(e) => {
                failed = Some(e);
                break;
            }
        }
    }
    // The comments that were posted are recorded even when a later one failed.
    let recorded = if heads.is_empty() {
        Ok(())
    } else {
        write_record(gh, &heads, &[], now)
    };
    if let Some(e) = failed {
        return Err(e.with_data(json!({"approved": approved})));
    }
    recorded.map_err(|e| e.with_data(json!({"approved": approved})))?;
    Ok(Outcome::data(json!({"approved": approved})))
}

/// Post `/bosun approve <sha>`, also when one that counts already exists: the new comment
/// re-runs risk-check if its run was lost. Returns the new comment's id and whether one existed.
fn approve_one(gh: &dyn Github, pr: u64, sha: &str, base: &str) -> Result<(u64, bool)> {
    let comments = gh.list_comments(pr)?;
    let existed = owner_logins(gh, base)?
        .iter()
        .any(|o| owner_comment(&comments, o, sha).is_some());
    Ok((
        gh.add_comment(pr, &format!("/bosun approve {sha}"))?.id,
        existed,
    ))
}

fn ready_json(r: &Ready) -> Value {
    json!({"pr": r.pr, "title": r.title, "head_sha": r.head_sha,
        "risk_from_status": r.risk, "files": r.files,
        "files_truncated": r.files.len() >= COMPARE_FILES_CAP})
}

/// Why `n` is not in the ready list.
fn why_not(gh: &dyn Github, b: &Board, n: u64) -> Result<String> {
    if let Some((_, why)) = b.waiting.iter().find(|(pr, _)| *pr == n) {
        return Ok((*why).to_string());
    }
    Ok(match gh.get_pull(n)? {
        None => "not found".into(),
        Some(p) if p.state != "open" => "closed".into(),
        Some(_) => "not waiting for an owner".into(),
    })
}

/// `bosun gate main-open --pr N`, posted with the owner token: the same `judge`. Only for a PR
/// that changes the main-open workflow (the base workflow cannot judge it). Exit 4 when main is
/// red and the PR is no mainfix; exit 3 when the PR is closed, missing or changes no such file.
fn post_main_open(gh: &dyn Github, n: u64) -> Result<PrHead> {
    let pull = gh
        .get_pull(n)?
        .filter(|p| p.state == "open")
        .ok_or_else(|| {
            BosunError::conflict(
                format!("PR #{n} is not an open PR"),
                "name an open PR; `bosun approve --list` lists the PRs that wait for an owner",
            )
        })?;
    if !gh
        .pull_files(n)?
        .iter()
        .any(|f| f.filename == MAIN_OPEN_WORKFLOW)
    {
        return Err(BosunError::conflict(
            format!("PR #{n} does not change {MAIN_OPEN_WORKFLOW}"),
            "the main-open workflow posts this status for other PRs; re-run it, or wait for it",
        ));
    }
    let red = main_red(gh)?;
    let mainfix = red.is_some() && is_mainfix(gh, &pull)?;
    let v = judge(red, mainfix);
    let state = if v.pass { "success" } else { "failure" };
    let description = format!("{} (posted by bosun approve --main-open)", v.description);
    gh.create_status(&pull.head_sha, "main-open", state, &description, None)?;
    if v.pass {
        return Ok(PrHead {
            pr: n,
            head_sha: pull.head_sha,
        });
    }
    let red = red.unwrap_or_default();
    Err(BosunError::halted(
        v.description,
        format!("fix main with a bosun:kind/mainfix PR, or wait for #{red} to close"),
    )
    .with_data(json!({"pr": n, "head_sha": pull.head_sha, "main_red": red, "pass": false})))
}

/// One `approve` record on the registry issue: an audit hint, never authority.
fn write_record(gh: &dyn Github, prs: &[PrHead], main_open: &[PrHead], at: &str) -> Result<()> {
    let (issue, _) = registry::ensure(gh)?;
    let record = Record::Approve {
        prs: prs.to_vec(),
        main_open: main_open.to_vec(),
        at: at.into(),
    };
    gh.add_comment(issue, &record.to_line())?;
    Ok(())
}

/// Belt and braces: a server that echoes the token in a message or a title never gets it to
/// stdout or stderr.
fn redact(token: &str, result: CmdResult) -> CmdResult {
    let scrub = |s: &str| s.replace(token, "[redacted]");
    let scrub_json = |v: Value| serde_json::from_str(&scrub(&v.to_string())).unwrap_or(Value::Null);
    match result {
        Ok(mut o) => {
            // `next` is not scrubbed: approve never sets it.
            debug_assert!(o.next.is_empty(), "approve returns no next items");
            o.data = scrub_json(o.data);
            Ok(o)
        }
        Err(mut e) => {
            e.message = scrub(&e.message);
            e.fix = scrub(&e.fix);
            e.data = e.data.map(scrub_json);
            Err(e)
        }
    }
}

/// `bosun approve <pr>@<head> --sign-key <pem>`: the signer (the orchestrator session, on the owner's
/// word in chat) signs `signed_message` for the PR's current head and posts the comment with
/// the session's own token. It works in a Claude cloud session: the signature, not the author,
/// is what risk-check checks. The key is never printed or logged.
fn sign(dir: Option<&Path>, args: &ApproveArgs, key_path: &Path) -> CmdResult {
    // The owner approves the head they saw: a bare PR number would sign whatever was pushed
    // in between.
    let (pr, pinned) = match args.prs.as_slice() {
        [target] => parse_target(target),
        _ => None,
    }
    .ok_or_else(|| {
        BosunError::usage(
            "--sign-key takes one <pr>@<40-hex head sha>: the head the owner approved",
            "run `bosun approve --list` (it works in the cloud), confirm the head with the owner, then `bosun approve <pr>@<head> --sign-key <pem>`",
        )
    })?;
    let api = std::env::var("BOSUN_GITHUB_API").unwrap_or_else(|_| "https://api.github.com".into());
    if !api_url_ok(&api) {
        return Err(BosunError::error(
            "BOSUN_GITHUB_API is not https://, so the head it reports is not trusted for signing",
            "unset BOSUN_GITHUB_API (or set it to an https:// URL)",
        ));
    }
    let pair = read_key(key_path)?;
    let app = App::load(dir)?;
    let gh = app.gh.as_ref();
    // Also for --print: a printed body could be posted by hand while bosun is halted.
    kill_switch(gh)?;
    let pull = gh
        .get_pull(pr)?
        .filter(|p| p.state == "open")
        .ok_or_else(|| {
            BosunError::conflict(format!("PR #{pr} is not an open PR"), "name an open PR")
        })?;
    let head = pull.head_sha.clone();
    if pinned != head {
        return Err(BosunError::conflict(
            format!("PR #{pr} head changed (now {head}); nothing was signed"),
            "show the owner the new head and sign it only on their word",
        ));
    }
    let role = match args.role.as_deref() {
        Some("delegate") => Role::Delegate,
        _ => Role::Owner,
    };
    let until = match args.bootstrap {
        true if role == Role::Delegate => {
            return Err(BosunError::usage(
                "--bootstrap signs with an owner key, not --as delegate",
                "drop --as delegate, or drop --bootstrap",
            ));
        }
        true => Some(bootstrap_until(gh, &app.ctx.now.to_string()[..10])?),
        false => None,
    };
    // A request for changes, and any withdrawal for a delegate, cannot be signed away. After a
    // dismissal an owner key signs v2, bound to each dismissed owner review (spec section 8).
    let mut reviews = vec![None];
    match withdrawn(gh, &pull)? {
        Some((_, true)) if role == Role::Owner && until.is_none() => {
            let all = gh.list_reviews(pr)?;
            reviews = owner_logins(gh, &pull.base_sha)?
                .iter()
                .filter_map(|o| dismissed_review(&all, o, &head))
                .map(Some)
                .collect();
            if reviews.is_empty() || reviews.contains(&Some(0)) {
                return Err(BosunError::conflict(
                    format!("PR #{pr}: the dismissed review cannot be named, nothing was signed"),
                    "the owner posts a plain `/bosun approve <head>` comment or submits a GitHub approving review",
                ));
            }
        }
        Some((why, _)) => {
            return Err(BosunError::conflict(
                format!("PR #{pr}: a signature would not count, nothing was signed: {why}"),
                "the owner submits a GitHub approving review (or, after a dismissal, posts a plain `/bosun approve <head>` comment); only an owner key signs after a dismissal, and a request for changes ends only with a review",
            ));
        }
        None => {}
    }
    let public = b64::encode(ring::signature::KeyPair::public_key(&pair).as_ref());
    let keys = enrolled_keys(gh).map_err(|why| {
        BosunError::error(
            format!("signed approvals are off: {why}"),
            format!(
                "fix {APPROVERS_PATH} on the default branch with a PR the owner approves on GitHub"
            ),
        )
    })?;
    let key = keys
        .into_iter()
        .find(|k| b64::decode(&k.ed25519).map(|b| b64::encode(&b)) == Some(public.clone()))
        .ok_or_else(|| {
            BosunError::error(
                format!("the key is not enrolled in {APPROVERS_PATH} on the default branch"),
                format!(
                    "enroll the public key {public} in {APPROVERS_PATH} with a PR the owner approves on GitHub"
                ),
            )
        })?;
    if key.role != role {
        return Err(BosunError::error(
            format!(
                "key {} is enrolled as {}, not {}; nothing was signed",
                key.id,
                key.role.name(),
                role.name()
            ),
            format!(
                "pass `--as {}`, or enroll the key with that role",
                key.role.name()
            ),
        ));
    }
    let key_id = key.id;
    let (owner, repo) = (&app.ctx.repo.owner, &app.ctx.repo.name);
    let mut signed = Vec::new();
    let note = until.as_deref().map(bootstrap_line);
    for after in reviews {
        let (form, msg) = match (after, &until) {
            (Some(r), _) => (Form::V2(r), signed_message_v2(owner, repo, pr, &head, r)),
            (None, Some(_)) => (
                Form::Bootstrap,
                signed_message_bootstrap(owner, repo, pr, &head),
            ),
            (None, None) => (Form::V1, signed_message(role, owner, repo, pr, &head)),
        };
        let sig = b64::encode(pair.sign(msg.as_bytes()).as_ref());
        let body = signed_comment(&head, form, note.as_deref(), &key_id, &sig);
        signed.push((after, body));
    }
    if args.print {
        let all: Vec<Value> = (signed.iter())
            .map(|(after, body)| json!({"after_review": after, "body": body}))
            .collect();
        return Ok(Outcome::data(
            json!({"pr": pr, "head_sha": head, "key_id": key_id, "role": role.name(),
                "bootstrap_until": until, "body": signed[0].1, "bodies": all}),
        ));
    }
    let mut approved = Vec::new();
    for (after, body) in signed {
        // The comments already posted stay listed when a later one fails.
        let id = match gh.add_comment(pr, &body) {
            Ok(c) => c.id,
            Err(e) => return Err(e.with_data(json!({"approved": approved}))),
        };
        let mut entry = json!({"pr": pr, "head_sha": head, "key_id": key_id,
            "role": role.name(), "comment_id": id});
        if let Some(r) = after {
            entry["after_review"] = json!(r);
        }
        approved.push(entry);
    }
    Ok(Outcome::data(json!({"approved": approved})))
}

/// The last day of bootstrap mode (D-012) from `[approvals] bootstrap_until` on the default
/// branch, which risk-check reads at the PR's base; an error when it is off or ended before `today`.
fn bootstrap_until(gh: &dyn Github, today: &str) -> Result<String> {
    let text = gh.file_at(CONFIG_FILE, &default_ref(gh)?)?;
    let until = match text {
        Some(t) => Config::parse(&t, CONFIG_FILE)?.approvals.bootstrap_until,
        None => None,
    };
    let fix = "nothing was signed: ask the owner with a form in the Claude Code chat and sign without --bootstrap on their answer (D-012)";
    match until {
        None => Err(BosunError::error(
            format!(
                "bootstrap mode is off: `[approvals] bootstrap_until` is not set in {CONFIG_FILE} on the default branch"
            ),
            fix,
        )),
        Some(d) if today > d.as_str() => Err(BosunError::error(
            format!("bootstrap mode ended on {d} (D-012)"),
            fix,
        )),
        Some(d) => Ok(d),
    }
}

/// An ed25519 key from a PKCS#8 PEM (`openssl genpkey -algorithm ed25519`). Errors name the
/// path only, never the content.
fn read_key(path: &Path) -> Result<ring::signature::Ed25519KeyPair> {
    let bad = || {
        BosunError::error(
            format!(
                "{} is not an ed25519 PKCS#8 PEM private key",
                path.display()
            ),
            "pass the file `openssl genpkey -algorithm ed25519 -out key.pem` wrote",
        )
    };
    let text = std::fs::read_to_string(path).map_err(|e| {
        BosunError::error(
            format!("cannot read {}: {}", path.display(), e.kind()),
            "pass a readable PEM file to --sign-key",
        )
    })?;
    #[cfg(unix)]
    if let Ok(meta) = std::fs::metadata(path) {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            eprintln!(
                "bosun: approve: {} is readable by group or others; chmod 600 it",
                path.display()
            );
        }
    }
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    if lines.next() != Some("-----BEGIN PRIVATE KEY-----") {
        return Err(bad());
    }
    let b64_body: String = lines
        .take_while(|l| *l != "-----END PRIVATE KEY-----")
        .collect();
    let der = b64::decode(&b64_body).ok_or_else(bad)?;
    ring::signature::Ed25519KeyPair::from_pkcs8_maybe_unchecked(&der).map_err(|_| bad())
}
