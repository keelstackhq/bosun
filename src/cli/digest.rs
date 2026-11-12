//! `bosun digest` (spec §14, §18): the owner's daily digest as one D-012 chat form, and with
//! `--answer` the owner's reply turned into `bosun approve` calls in `next`. It writes nothing
//! and signs no approval: the orchestrator, the key holder, runs those calls. A raise counts only
//! signed with an enrolled owner key (`--sign-key`), as `answer --chat`.

use std::path::Path;

use serde_json::{Value, json};

use super::budget::{feature_levels, knob_from_default};
use super::{ApproveArgs, DigestArgs};
use crate::app::{App, kill_switch, main_red};
use crate::cli::approve::read_key;
use crate::cli::risk_check::enrolled_keys;
use crate::core::answer::text_sha256;
use crate::core::approve::{APPROVERS_PATH, Role};
use crate::core::digest::{Item, digest_with, parse_header, parse_reply, pick, raise_message};
use crate::core::mode::BudgetFeature;
use crate::gates::main_open::is_mainfix;
use crate::github::{IssueState, b64};
use crate::output::{BosunError, CmdResult, Outcome};

const NEEDS_HUMAN: &str = "bosun:state/needs-human";
const BYPASS: &str = "mainfix: approve it now, not in the digest";
const HEADER_FIX: &str = "put data.header of the digest the owner answered as the first line, then the owner's reply verbatim";

fn read_text(from: &str) -> Result<String, BosunError> {
    let text = if from == "-" {
        std::io::read_to_string(std::io::stdin())
    } else {
        std::fs::read_to_string(from)
    };
    text.map_err(|e| {
        BosunError::usage(
            format!("cannot read {from}: {e}"),
            "pass the reply on stdin or with --from FILE",
        )
    })
}

/// From `approve --list`: owner-waiting PRs, mainfix PRs (only while main is red) and what a
/// tokenless listing could not read.
type Board = (Vec<Item>, Vec<u64>, Value);

fn board(dir: Option<&Path>, app: &App) -> Result<Board, BosunError> {
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
    // Mainfix PRs skip the batch only while main is red, as main-open judges them.
    let red = main_red(app.gh.as_ref())?.is_some();
    let pulls = if red {
        app.gh.list_pulls(None, IssueState::Open)?
    } else {
        Vec::new()
    };
    let (mut items, mut bypass) = (Vec::new(), Vec::new());
    for r in data["ready"].as_array().into_iter().flatten() {
        let pr = r["pr"].as_u64().unwrap_or_default();
        let strs = |v: &Value| -> Vec<String> {
            let a = v.as_array().into_iter().flatten();
            a.filter_map(|s| s.as_str().map(String::from)).collect()
        };
        if let Some(p) = pulls.iter().find(|p| p.number == pr)
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
    Ok((items, bypass, data["skipped"].clone()))
}

fn repo(app: &App) -> String {
    let r = &app.ctx.repo;
    format!("{}/{}", r.owner, r.name).to_ascii_lowercase()
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
    let (items, bypass, skipped) = board(dir, &app)?;
    let mut asks = Vec::new();
    for i in app.gh.list_issues(&[NEEDS_HUMAN], IssueState::Open)? {
        if !i.pull_request {
            asks.push((i.number, i.title));
        }
    }
    let day = app.ctx.now.to_string();
    // Budget asks (knob `ask`): the indexed epics at 100 % of their feature budget.
    let mut over = Vec::new();
    if knob_from_default(app.gh.as_ref())? == BudgetFeature::Ask {
        for l in feature_levels(&app)? {
            if l["level"] == "over" {
                let f = |k: &str| l[k].as_f64().unwrap_or_default();
                over.push((
                    l["epic"].as_u64().unwrap_or_default(),
                    f("spent_usd"),
                    f("budget_usd"),
                ));
            }
        }
    }
    let d = digest_with(&repo(&app), &day[..10], &items, &asks, &over);
    Ok(Outcome::data(json!({
        "message_id": d.id,
        "message": d.text,
        "header": d.header,
        "items": items.iter().map(|i| json!({"pr": i.pr, "head_sha": i.head, "rules": i.rules,
            "title": i.title})).collect::<Vec<_>>(),
        "asks": asks.iter().map(|(n, _)| json!({"issue": n,
            "answer": format!("bosun answer {n} --chat")})).collect::<Vec<_>>(),
        "bypass": bypass.iter().map(|n| json!({"pr": n, "why": BYPASS})).collect::<Vec<_>>(),
        "budget_asks": over.iter().map(|(e, s, b)| json!({"epic": e, "spent_usd": s,
            "budget_usd": b})).collect::<Vec<_>>(),
        "skipped": skipped,
    })))
}

fn answer(dir: Option<&Path>, app: &App, args: &DigestArgs) -> CmdResult {
    kill_switch(app.gh.as_ref())?;
    let text = read_text(&args.from)?;
    super::refuse_secret("the reply", &text)?;
    let mut lines = text.lines().skip_while(|l| l.trim().is_empty());
    let header = lines.next().unwrap_or_default();
    let (hrepo, day, id, shown) = parse_header(header).ok_or_else(|| {
        BosunError::usage("the answer does not start with a digest header", HEADER_FIX)
    })?;
    if hrepo != repo(app) {
        return Err(BosunError::usage(
            format!("the digest header is for {hrepo}, not {}", repo(app)),
            HEADER_FIX,
        ));
    }
    let body = lines.collect::<Vec<_>>().join("\n");
    let reply = parse_reply(&body);
    // The owner quotes the id they saw, so a reply never pairs with another digest's heads.
    if reply.ids.is_empty() {
        return Err(BosunError::usage(
            "the owner's reply quotes no digest id",
            "relay the owner's reply as typed from the form (`<id> approve ...`); ask the owner to reply with the id",
        ));
    }
    if let Some(quoted) = reply.ids.iter().find(|q| **q != id) {
        return Err(BosunError::conflict(
            format!("the owner answered digest {quoted}, the header is digest {id}"),
            "pass the header of the digest the owner answered (its id is in the message's first line)",
        ));
    }
    // One raise per epic in a reply: two amounts for one epic are the owner's to settle.
    if let Some(w) = reply.raise.windows(2).find(|w| w[0].0 == w[1].0) {
        return Err(BosunError::usage(
            format!(
                "the reply raises epic {} twice (+{} and +{})",
                w[0].0, w[0].1, w[1].1
            ),
            "ask the owner for one `raise <epic> +<usd>` per epic and relay that reply",
        ));
    }
    let now: Vec<(u64, String)> = board(dir, app)?
        .0
        .iter()
        .map(|i| (i.pr, i.head.clone()))
        .collect();
    let (mut approve, mut dropped) = (Vec::new(), Vec::new());
    for pr in &reply.approve {
        match pick(&shown, &now, *pr) {
            Ok(h) => approve.push((*pr, h)),
            Err(why) => dropped.push(json!({"pr": pr, "why": why})),
        }
    }
    let (mut raises, mut refused) = (Vec::new(), Vec::new());
    // A given key is always checked: an unreadable key or approvers file is an error, a key that
    // is not an owner's refuses the raises.
    let key = match &args.sign_key {
        Some(path) => owner_key(app, path)?,
        None => Err("not signed: run again with --sign-key and an enrolled owner key".to_string()),
    };
    let (owner, name) = (&app.ctx.repo.owner, &app.ctx.repo.name);
    let sha = text_sha256(&body);
    for (epic, usd) in &reply.raise {
        match &key {
            Ok((key_id, pair)) => {
                let message = raise_message(owner, name, (&id, &day, &sha), *epic, *usd);
                let sig = b64::encode(pair.sign(message.as_bytes()).as_ref());
                raises.push(json!({"epic": epic, "usd": usd, "key_id": key_id,
                    "message": message, "sig": sig}));
            }
            Err(why) => refused.push(json!({"raise": {"epic": epic, "usd": usd}, "why": why})),
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

type Key = Result<(String, ring::signature::Ed25519KeyPair), String>;

/// The enrolled owner key id of the key at `path` and the key, or why it signs nothing.
fn owner_key(app: &App, path: &Path) -> Result<Key, BosunError> {
    let keys = enrolled_keys(app.gh.as_ref()).map_err(|why| {
        BosunError::error(
            format!("signed raises are off: {why}"),
            format!("enroll the orchestrator's key in {APPROVERS_PATH} on the default branch with a PR the owner approves on GitHub"),
        )
    })?;
    let pair = read_key(path)?;
    let public = b64::encode(ring::signature::KeyPair::public_key(&pair).as_ref());
    let key = keys.iter().filter(|k| k.role == Role::Owner).find(|k| {
        b64::decode(&k.ed25519).map(|b| b64::encode(&b)).as_deref() == Some(public.as_str())
    });
    Ok(match key {
        Some(k) => Ok((k.id.clone(), pair)),
        None => Err(format!(
            "the key {public} is not an owner key in {APPROVERS_PATH} on the default branch"
        )),
    })
}
