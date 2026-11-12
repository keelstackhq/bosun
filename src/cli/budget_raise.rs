//! `bosun budget raise`: record the owner-signed raises of `digest --answer` on their epics
//! (spec §5, modes 7c). The guard re-verifies every raise comment, so a forged one adds nothing.

use std::path::Path;

use serde_json::{Value, json};

use super::BudgetRaiseArgs;
use super::budget::{same_path, source};
use crate::app::{App, Result, kill_switch, trusted_notes};
use crate::cli::risk_check::enrolled_keys;
use crate::core::approve::{ApproverKey, Role, verify_ed25519};
use crate::core::digest::{RaiseMsg, parse_raise_message};
use crate::core::records::{Record, parse_record};
use crate::github::b64;
use crate::output::{BosunError, CmdResult, Outcome};
use crate::registry;
use crate::time::{Timestamp, parse_rfc3339};

type Why<T> = std::result::Result<T, String>;

/// A raise entry's claim `(epic, usd, key id)` and `(message, sig)`, checked against the enrolled
/// keys, the repo and the day (at most two days before `at`, never after it).
pub(super) fn verify(
    keys: &[ApproverKey],
    repo: &str,
    at: Timestamp,
    claim: (u64, u64, &str),
    (message, sig): (&str, &str),
) -> Why<RaiseMsg> {
    let m = parse_raise_message(message).ok_or("the message is not a bosun-raise v1 message")?;
    if m.repo != repo {
        return Err(format!("the message is for repo {}, not {repo}", m.repo));
    }
    if (m.epic, m.usd) != (claim.0, claim.1) {
        return Err("the message does not match the entry's epic and usd".into());
    }
    let key = keys
        .iter()
        .find(|k| k.id == claim.2 && k.role == Role::Owner)
        .ok_or("the key is not enrolled as an owner key")?;
    let ok = match (b64::decode(&key.ed25519), b64::decode(sig)) {
        (Some(p), Some(s)) => verify_ed25519(&p, message.as_bytes(), &s),
        _ => false,
    };
    if !ok {
        return Err("bad signature".into());
    }
    let day = parse_rfc3339(&format!("{}T00:00:00Z", m.day)).ok_or("the day is not a date")?;
    let today = at.day_start().0;
    match day.0 {
        d if d > today => Err("the digest day is in the future".into()),
        d if d < today - 2 * 86_400 => Err("the digest day is stale (over 2 days old)".into()),
        _ => Ok(m),
    }
}

fn repo_of(app: &App) -> String {
    let r = &app.ctx.repo;
    format!("{}/{}", r.owner, r.name).to_ascii_lowercase()
}

/// An accepted raise: its digest id, amount and digest day (days since the epoch).
pub(super) struct Raise {
    pub digest: String,
    pub usd: u64,
    pub day: i64,
}

fn day_no(day: &str) -> Option<i64> {
    Some(
        parse_rfc3339(&format!("{day}T00:00:00Z"))?
            .0
            .div_euclid(86_400),
    )
}

/// Why a raise of `usd` on `day` is not accepted beside `have`: one raise a day, and the raises
/// of the days `day + span.0 ..= day + span.1` plus this one stay within the base budget.
fn capped(have: &[Raise], day: i64, usd: u64, base: f64, span: (i64, i64)) -> Option<String> {
    if have.iter().any(|h| h.day == day) {
        return Some("a raise is already counted for that digest day (one a day)".into());
    }
    let sum = |f: &dyn Fn(&Raise) -> bool| {
        have.iter()
            .filter(|h| f(h))
            .fold(0u64, |a, h| a.saturating_add(h.usd))
    };
    let all = sum(&|_| true);
    if all.saturating_add(usd) as f64 > 2.0 * base {
        return Some(format!(
            "+{usd} is over the cap: raises in all total ${all}+{usd} against twice the base budget ${base:.2} (more needs an R-gov plan PR)"
        ));
    }
    let near = sum(&|h| (day + span.0..=day + span.1).contains(&h.day));
    (near.saturating_add(usd) as f64 > base).then(|| {
        format!(
            "+{usd} is over the cap: raises within 7 days total ${near}+{usd} against the base budget ${base:.2} (more needs an R-gov plan PR)"
        )
    })
}

/// How many dollars a raise signed on digest day `today` may still add to `base` without
/// `bosun budget raise` refusing it: nothing when one is already counted that day, else the
/// room left in the 7-day cap (raises within 6 days either side) and the lifetime cap.
pub(super) fn room(have: &[Raise], base: f64, today: &str) -> u64 {
    let Some(day) = day_no(today) else { return 0 };
    if have.iter().any(|h| h.day == day) {
        return 0;
    }
    let sum = |near: bool| {
        have.iter()
            .filter(|h| !near || (day - 6..=day + 6).contains(&h.day))
            .fold(0u64, |a, h| a.saturating_add(h.usd)) as f64
    };
    (base - sum(true)).min(2.0 * base - sum(false)).max(0.0) as u64
}

/// The valid raises on `epic` in (day, comment) order: verified, one per digest id and per day,
/// and every 7 days at most the base budget in all.
pub(super) fn valid_raises(app: &App, epic: u64, base: f64) -> Result<Vec<Raise>> {
    let keys = enrolled_keys(app.gh.as_ref()).unwrap_or_default();
    let repo = repo_of(app);
    let mut found: Vec<(i64, u64, Raise)> = Vec::new();
    for c in trusted_notes(app.gh.as_ref(), epic)? {
        let Some(Record::Raise {
            epic: e,
            usd,
            key_id,
            message,
            sig,
            ..
        }) = parse_record(&c.body)
        else {
            continue;
        };
        // An unreadable creation time is 1970: the day check fails closed.
        let claim = (epic, usd, key_id.as_str());
        let Ok(m) = verify(&keys, &repo, c.created_at, claim, (&message, &sig)) else {
            continue;
        };
        if let (true, Some(day)) = (e == epic, day_no(&m.day)) {
            found.push((
                day,
                c.id,
                Raise {
                    digest: m.digest,
                    usd,
                    day,
                },
            ));
        }
    }
    found.sort_by_key(|f| (f.0, f.1));
    let mut out: Vec<Raise> = Vec::new();
    for (_, _, r) in found {
        if !out.iter().any(|o| o.digest == r.digest)
            && capped(&out, r.day, r.usd, base, (-6, 0)).is_none()
        {
            out.push(r);
        }
    }
    Ok(out)
}

/// Whether the registry's first index of some (plan, item) names `epic`.
fn indexed(app: &App, epic: u64) -> Result<bool> {
    let notes = registry::load(app.gh.as_ref())?.map(|r| r.1);
    let mut seen: Vec<(String, String)> = vec![];
    for c in notes.unwrap_or_default() {
        let Some(Record::EpicIndex {
            plan,
            item,
            epic: e,
            ..
        }) = parse_record(&c.body)
        else {
            continue;
        };
        if seen.iter().any(|s| same_path(&s.0, &plan) && s.1 == item) {
            continue;
        }
        if e == epic {
            return Ok(true);
        }
        seen.push((plan, item));
    }
    Ok(false)
}

fn entries(from: &str) -> Result<Vec<Value>> {
    let v = super::budget::read_input(from)?;
    let list = v.get("data").unwrap_or(&v).get("raises").unwrap_or(&v);
    list.as_array().cloned().ok_or_else(|| {
        BosunError::usage(
            "the input has no raises array",
            "pass the output of `bosun digest --answer --sign-key <pem>` with --from -",
        )
    })
}

/// Check one entry and write its raise: `(epic, usd, digest)`, or why it was refused. GitHub
/// errors are the command's own.
fn one(app: &App, keys: &[ApproverKey], e: &Value) -> Result<Why<(u64, u64, String)>> {
    let s = |k: &str| e[k].as_str().unwrap_or_default();
    let Some((epic, usd)) = e["epic"].as_u64().zip(e["usd"].as_u64()) else {
        return Ok(Err("the entry has no epic and usd".into()));
    };
    let claim = (epic, usd, s("key_id"));
    let at = app.ctx.now;
    let m = match verify(keys, &repo_of(app), at, claim, (s("message"), s("sig"))) {
        Ok(m) => m,
        Err(why) => return Ok(Err(why)),
    };
    if !indexed(app, epic)? {
        return Ok(Err(format!(
            "epic #{epic} is not in the epic index (the orchestrator runs `bosun plan amend <plan>`)"
        )));
    }
    let base = source(app, epic)?.budget;
    let have = valid_raises(app, epic, base)?;
    if have.iter().any(|h| h.digest == m.digest) {
        return Ok(Err(format!(
            "digest {} is already counted on #{epic}",
            m.digest
        )));
    }
    // Wider than the read-time window, so a raise recorded now is never dropped later.
    let day = day_no(&m.day).unwrap_or_default();
    if let Some(why) = capped(&have, day, usd, base, (-6, 6)) {
        return Ok(Err(why));
    }
    let r = Record::Raise {
        session: app.ctx.session.clone(),
        epic,
        usd,
        digest: m.digest.clone(),
        day: m.day,
        key_id: s("key_id").into(),
        message: s("message").into(),
        sig: s("sig").into(),
    };
    app.gh.add_comment(epic, &r.to_line())?;
    Ok(Ok((epic, usd, m.digest)))
}

pub fn run(dir: Option<&Path>, a: &BudgetRaiseArgs) -> CmdResult {
    let app = App::load(dir)?;
    kill_switch(app.gh.as_ref())?;
    let list = entries(&a.from)?;
    let keys = enrolled_keys(app.gh.as_ref()).map_err(|why| {
        BosunError::error(
            format!("raises cannot be checked: {why}"),
            "fix or add `.bosun/approvers.toml` on the default branch (an owner key)",
        )
    })?;
    let (mut recorded, mut refused) = (vec![], vec![]);
    for e in &list {
        match one(&app, &keys, e)? {
            Ok((epic, usd, digest)) => {
                recorded.push(json!({"epic": epic, "usd": usd, "digest": digest}));
            }
            Err(why) => {
                let (epic, usd) = (e["epic"].as_u64(), e["usd"].as_u64());
                refused.push(json!({"epic": epic, "usd": usd, "why": why}));
            }
        }
    }
    Ok(Outcome::data(
        json!({"recorded": recorded, "refused": refused}),
    ))
}
