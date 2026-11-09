//! `bosun budget report`: one month of spend per PR (spec §9). Pure: rows in, JSON and text out.
//! The output holds no clock reading, so the same input gives the same bytes.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::cost::{per_task, round_usd};
use super::records::{Note, Record, parse_record};
use crate::time::parse_rfc3339;

/// One PR with recorded spend.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub pr: u64,
    pub title: Option<String>,
    pub issue: u64,
    pub sessions: Vec<String>,
    pub models: Vec<String>,
    pub usd: f64,
    /// No record carries tokens yet, so this is always `None`: unknown is never guessed.
    pub tokens: Option<u64>,
    pub outcome: &'static str,
    pub merged_at: Option<String>,
}

/// `YYYY-MM` with a month from 01 to 12, else `None`.
pub fn parse_month(s: &str) -> Option<String> {
    let (y, m) = s.split_once('-')?;
    let ok = y.len() == 4 && m.len() == 2 && s.bytes().all(|b| b == b'-' || b.is_ascii_digit());
    (ok && (1..=12).contains(&m.parse::<u32>().ok()?)).then(|| s.to_string())
}

/// True when the RFC 3339 time `at` falls in the UTC month `month` (`YYYY-MM`).
pub fn in_month(month: &str, at: &str) -> bool {
    parse_rfc3339(at).is_some_and(|t| t.to_string().starts_with(month))
}

/// `held` is merged and not undone by a merged revert PR; `closed` is closed without a merge.
pub fn outcome(state: &str, merged: bool, reverted: bool) -> &'static str {
    match (merged, reverted, state) {
        (true, true, _) => "reverted",
        (true, false, _) => "held",
        (false, _, "closed") => "closed",
        _ => "open",
    }
}

/// What the records on a task and its PR say: sorted session ids, sorted models, the spend sum
/// and the number of spend records.
pub fn facts(notes: &[Note]) -> (Vec<String>, Vec<String>, f64, usize) {
    let (mut sessions, mut models) = (BTreeSet::new(), BTreeSet::new());
    let (mut usd, mut n) = (0.0, 0);
    for note in notes {
        match parse_record(&note.body) {
            Some(Record::Spend {
                session, usd: u, ..
            }) if u.is_finite() && u >= 0.0 => {
                sessions.insert(session);
                usd += u;
                n += 1;
            }
            Some(Record::Verdict { model: Some(m), .. }) => {
                models.insert(m);
            }
            Some(Record::Escalate { to, .. }) => {
                models.insert(to);
            }
            _ => {}
        }
    }
    (
        sessions.into_iter().collect(),
        models.into_iter().collect(),
        round_usd(usd),
        n,
    )
}

fn totals(rows: &[Row]) -> Value {
    let count = |o: &str| rows.iter().filter(|r| r.outcome == o).count();
    let (held, reverted) = (count("held"), count("reverted"));
    let usd = round_usd(rows.iter().map(|r| r.usd).sum());
    let mut top: Vec<&Row> = rows.iter().collect();
    top.sort_by(|a, b| b.usd.total_cmp(&a.usd).then(a.pr.cmp(&b.pr)));
    let top: Vec<Value> = top
        .iter()
        .take(5)
        .map(|r| json!({"pr": r.pr, "title": r.title, "usd": r.usd}))
        .collect();
    json!({
        "prs": rows.len(), "usd": usd,
        "merged": held + reverted, "held": held, "reverted": reverted,
        "cost_per_merged_usd": per_task(usd, held + reverted),
        "cost_per_held_usd": per_task(usd, held),
        "top_prs": top,
    })
}

/// The report data: `month`, `rows` sorted by PR, `totals`.
pub fn report(month: &str, rows: &[Row]) -> Value {
    let mut rows = rows.to_vec();
    rows.sort_by_key(|r| r.pr);
    let list: Vec<Value> = rows
        .iter()
        .map(|r| {
            json!({"pr": r.pr, "title": r.title, "issue": r.issue, "sessions": r.sessions,
                "models": r.models, "usd": r.usd, "tokens": r.tokens,
                "outcome": r.outcome, "merged_at": r.merged_at})
        })
        .collect();
    json!({"month": month, "rows": list, "totals": totals(&rows)})
}

fn usd_cell(v: &Value) -> String {
    v.as_f64().map_or("-".into(), |x| format!("{x:.2}"))
}

fn cell(v: &Value) -> String {
    match v {
        Value::Null => "-".into(),
        Value::String(s) => s.replace('|', "\\|").replace('\n', " "),
        Value::Array(a) if a.is_empty() => "-".into(),
        Value::Array(a) => a.iter().map(cell).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

/// A readable page of `report(..)`: totals, then one table row per PR.
pub fn markdown(data: &Value) -> String {
    let (t, month) = (&data["totals"], data["month"].as_str().unwrap_or(""));
    let mut out = format!("# Budget report {month}\n\n");
    out += &format!(
        "- PRs: {}\n- Spend: ${}\n- Merged: {} (held {}, reverted {})\n",
        t["prs"],
        usd_cell(&t["usd"]),
        t["merged"],
        t["held"],
        t["reverted"],
    );
    out += &format!(
        "- Cost per merged PR: ${}\n- Cost per held PR: ${}\n",
        usd_cell(&t["cost_per_merged_usd"]),
        usd_cell(&t["cost_per_held_usd"]),
    );
    out += "\n## Top PRs by spend\n\n";
    for p in t["top_prs"].as_array().into_iter().flatten() {
        out += &format!(
            "- #{} {}: ${}\n",
            p["pr"],
            cell(&p["title"]),
            usd_cell(&p["usd"])
        );
    }
    out += "\n## PRs\n\n| PR | Title | Task | Sessions | Models | USD | Tokens | Outcome | Merged at |\n";
    out += "| --- | --- | --- | --- | --- | ---: | ---: | --- | --- |\n";
    for r in data["rows"].as_array().into_iter().flatten() {
        out += &format!(
            "| #{} | {} | #{} | {} | {} | {} | {} | {} | {} |\n",
            r["pr"],
            cell(&r["title"]),
            r["issue"],
            cell(&r["sessions"]),
            cell(&r["models"]),
            usd_cell(&r["usd"]),
            cell(&r["tokens"]),
            cell(&r["outcome"]),
            cell(&r["merged_at"]),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::Timestamp;

    fn row(pr: u64, usd: f64, outcome: &'static str) -> Row {
        Row {
            pr,
            title: Some(format!("T{pr}")),
            issue: pr + 100,
            sessions: vec!["s".into()],
            models: vec![],
            usd,
            tokens: None,
            outcome,
            merged_at: None,
        }
    }

    fn note(body: &str) -> Note {
        Note {
            id: 1,
            body: body.into(),
            created_at: Timestamp(0),
        }
    }

    #[test]
    fn month_parses_and_bounds() {
        assert_eq!(parse_month("2026-11").as_deref(), Some("2026-11"));
        for bad in ["2026-13", "2026-00", "26-11", "2026-1", "2026-11-01", "x"] {
            assert_eq!(parse_month(bad), None, "{bad}");
        }
        assert!(in_month("2026-11", "2026-11-30T23:59:59Z"));
        assert!(!in_month("2026-11", "2026-12-01T00:00:00Z"));
        assert!(!in_month("2026-11", "2026-11-01T01:00:00+02:00"));
        assert!(!in_month("2026-11", "junk"));
    }

    #[test]
    fn outcomes() {
        assert_eq!(outcome("closed", true, false), "held");
        assert_eq!(outcome("closed", true, true), "reverted");
        assert_eq!(outcome("closed", false, false), "closed");
        assert_eq!(outcome("open", false, false), "open");
    }

    #[test]
    fn facts_sum_spend_and_collect_models() {
        let spend = |s: &str, u: f64| {
            note(&format!(
                r#"{{"bosun":"spend","session":"{s}","usd":{u},"at":"2026-11-01T00:00:00Z"}}"#
            ))
        };
        let v = note(
            r#"{"bosun":"verdict","session":"r","head_sha":"a","approved":true,"model":"opus"}"#,
        );
        let (s, m, usd, n) = facts(&[spend("b", 0.1), spend("a", 0.2), spend("a", 1.0), v]);
        assert_eq!(
            (s, m, usd, n),
            (vec!["a".into(), "b".into()], vec!["opus".into()], 1.3, 3)
        );
    }

    #[test]
    fn totals_and_order() {
        let rows = [
            row(3, 1.0, "held"),
            row(1, 3.0, "reverted"),
            row(2, 2.0, "open"),
            row(4, 1.0, "held"),
            row(5, 0.5, "closed"),
            row(6, 0.25, "held"),
        ];
        let d = report("2026-11", &rows);
        let prs: Vec<u64> = d["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["pr"].as_u64().unwrap())
            .collect();
        assert_eq!(prs, [1, 2, 3, 4, 5, 6]);
        let t = &d["totals"];
        assert_eq!(
            (t["prs"].as_u64(), t["merged"].as_u64(), t["held"].as_u64()),
            (Some(6), Some(4), Some(3))
        );
        assert_eq!(t["usd"], 7.75);
        assert_eq!(t["cost_per_merged_usd"], 1.9375);
        let top: Vec<u64> = t["top_prs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["pr"].as_u64().unwrap())
            .collect();
        assert_eq!(top, [1, 2, 3, 4, 5]);
        assert!(d["rows"][0]["tokens"].is_null());
    }

    #[test]
    fn no_merged_means_null_costs_and_same_input_same_bytes() {
        let d = report("2026-11", &[row(1, 1.0, "open")]);
        assert!(d["totals"]["cost_per_merged_usd"].is_null());
        assert!(d["totals"]["cost_per_held_usd"].is_null());
        assert_eq!(
            markdown(&d),
            markdown(&report("2026-11", &[row(1, 1.0, "open")]))
        );
        assert!(markdown(&d).contains("| #1 | T1 | #101 | s | - | 1.00 | - | open | - |"));
    }
}
