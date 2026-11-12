//! `bosun digest` (spec §14, §18): the owner's daily digest as one D-012 chat form, and with
//! `--answer` the owner's reply turned into `bosun approve` calls in `next`. It writes nothing
//! and signs no approval: the orchestrator, the key holder, runs those calls. A raise counts only
//! signed with an enrolled owner key (`--sign-key`), as `answer --chat`.

use std::io::Read;
use std::path::Path;

use serde_json::{Value, json};

use super::{ApproveArgs, DigestArgs};
use crate::app::App;
use crate::cli::approve::read_key;
use crate::cli::risk_check::enrolled_keys;
use crate::core::approve::{APPROVERS_PATH, Role};
use crate::core::digest::{Item, digest, drop_reason, parse_header, parse_reply, raise_message};
use crate::gates::main_open::is_mainfix;
use crate::github::{IssueState, b64};
use crate::output::{BosunError, CmdResult, Outcome};

const NEEDS_HUMAN: &str = "bosun:state/needs-human";
const BYPASS: &str = "mainfix: approve it now, not in the digest";
const HEADER_FIX: &str = "put data.header of the digest the owner answered as the first line, then the owner's reply verbatim";

fn read_text(from: &str) -> Result<String, BosunError> {
    let mut text = String::new();
    let res = if from == "-" {
        std::io::stdin().read_to_string(&mut text).map(|_| ())
    } else {
        std::fs::read_to_string(from).map(|t| text = t)
    };
    res.map_err(|e| {
        BosunError::usage(
            format!("cannot read the answer from {from}: {e}"),
            "pass the reply on stdin or with --from FILE",
        )
    })?;
    Ok(text)
}

/// The owner-waiting PRs (mainfix PRs apart) and the mainfix PRs, from `approve --list`.
fn board(dir: Option<&Path>, app: &App) -> Result<(Vec<Item>, Vec<u64>), BosunError> {
    let list = ApproveArgs {
        prs: Vec::new(),
        list: true,
        main_open: None,
        sign_key: None,
        print: false,
        role: None,
        bootstrap: false,
    };
    let data = super::approve::run(dir, &list)?.data;
    let pulls = app.gh.list_pulls(None, IssueState::Open)?;
    let (mut items, mut bypass) = (Vec::new(), Vec::new());
    for r in data["ready"].as_array().into_iter().flatten() {
        let pr = r["pr"].as_u64().unwrap_or_default();
        let strs = |v: &Value| -> Vec<String> {
            let a = v.as_array().into_iter().flatten();
            a.filter_map(|s| s.as_str().map(String::from)).collect()
        };
        let pull = pulls.iter().find(|p| p.number == pr);
        if let Some(p) = pull
            && is_mainfix(app.gh.as_ref(), p)?
        {
            bypass.push(pr);
            continue;
        }
        items.push(Item {
            pr,
            head: r["head_sha"].as_str().unwrap_or_default().to_string(),
            rules: strs(&r["risk_from_status"]),
            title: r["title"].as_str().unwrap_or_default().to_string(),
        });
    }
    Ok((items, bypass))
}

pub fn run(dir: Option<&Path>, args: &DigestArgs) -> CmdResult {
    if args.sign_key.is_some() && !args.answer {
        return Err(BosunError::usage(
            "--sign-key signs the raises of an answer",
            "pass --answer with --sign-key",
        ));
    }
    let app = App::load(dir)?;
    if args.answer {
        return answer(dir, &app, args);
    }
    let (items, bypass) = board(dir, &app)?;
    let mut asks = Vec::new();
    for i in app.gh.list_issues(&[NEEDS_HUMAN], IssueState::Open)? {
        if !i.pull_request {
            asks.push((i.number, i.title));
        }
    }
    let d = digest(&items, &asks);
    Ok(Outcome::data(json!({
        "message_id": d.id,
        "message": d.text,
        "header": d.header,
        "items": items.iter().map(|i| json!({"pr": i.pr, "head_sha": i.head, "rules": i.rules,
            "title": i.title})).collect::<Vec<_>>(),
        "asks": asks.iter().map(|(n, _)| json!({"issue": n,
            "answer": format!("bosun answer {n} --chat")})).collect::<Vec<_>>(),
        "bypass": bypass.iter().map(|n| json!({"pr": n, "why": BYPASS})).collect::<Vec<_>>(),
        "budget_asks": [],
    })))
}

fn answer(dir: Option<&Path>, app: &App, args: &DigestArgs) -> CmdResult {
    let text = read_text(&args.from)?;
    super::refuse_secret("the reply", &text)?;
    let mut lines = text.lines().skip_while(|l| l.trim().is_empty());
    let header = lines.next().unwrap_or_default();
    let (id, shown) = parse_header(header).ok_or_else(|| {
        BosunError::usage("the answer does not start with a digest header", HEADER_FIX)
    })?;
    let reply = parse_reply(&lines.collect::<Vec<_>>().join("\n"));
    let (items, _) = board(dir, app)?;
    let now: Vec<(u64, String)> = items.iter().map(|i| (i.pr, i.head.clone())).collect();
    let (mut approve, mut dropped) = (Vec::new(), Vec::new());
    for pr in &reply.approve {
        match drop_reason(&shown, &now, *pr) {
            None => approve.push((*pr, shown.iter().find(|s| s.0 == *pr).unwrap().1.clone())),
            Some(why) => dropped.push(json!({"pr": pr, "why": why})),
        }
    }
    let (mut raises, mut refused) = (Vec::new(), Vec::new());
    if let Some(path) = &args.sign_key
        && !reply.raise.is_empty()
    {
        let (key_id, pair) = owner_key(app, path)?;
        let (owner, repo) = (&app.ctx.repo.owner, &app.ctx.repo.name);
        for (epic, usd) in &reply.raise {
            let message = raise_message(owner, repo, &id, *epic, *usd);
            let sig = b64::encode(pair.sign(message.as_bytes()).as_ref());
            raises.push(
                json!({"epic": epic, "usd": usd, "key_id": key_id, "message": message,
                "sig": sig}),
            );
        }
    } else {
        for (epic, usd) in &reply.raise {
            refused.push(json!({"raise": {"epic": epic, "usd": usd},
                "why": "not signed: run again with --sign-key and an enrolled owner key"}));
        }
    }
    let mut out = Outcome::data(json!({
        "message_id": id,
        "approve": approve.iter().map(|(pr, h)| json!({"pr": pr, "head_sha": h}))
            .collect::<Vec<_>>(),
        "dropped": dropped,
        "raises": raises,
        "refused": refused,
        "unknown": reply.unknown,
    }));
    for (pr, h) in &approve {
        let command = format!("bosun approve {pr}@{h} --sign-key <pem>");
        out = out.with_next("Bash", json!({"command": command}));
    }
    Ok(out)
}

/// The enrolled owner key id of the key at `path`, and the key; nothing is signed otherwise.
fn owner_key(
    app: &App,
    path: &Path,
) -> Result<(String, ring::signature::Ed25519KeyPair), BosunError> {
    let keys = enrolled_keys(app.gh.as_ref()).map_err(|why| {
        BosunError::error(
            format!("signed raises are off: {why}"),
            format!("enroll the orchestrator's key in {APPROVERS_PATH} on the default branch with a PR the owner approves on GitHub"),
        )
    })?;
    let pair = read_key(path)?;
    let public = b64::encode(ring::signature::KeyPair::public_key(&pair).as_ref());
    let key = keys
        .iter()
        .filter(|k| k.role == Role::Owner)
        .find(|k| b64::decode(&k.ed25519).map(|b| b64::encode(&b)) == Some(public.clone()))
        .ok_or_else(|| {
            BosunError::error(
                format!("the key is not an owner key in {APPROVERS_PATH} on the default branch; nothing was signed"),
                format!("enroll the public key {public} as an owner key in {APPROVERS_PATH} with a PR the owner approves on GitHub"),
            )
        })?;
    Ok((key.id.clone(), pair))
}
