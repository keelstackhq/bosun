//! The reviewer's verdict (spec §7): shape, validation and the rule that decides approval.

use crate::core::meta::linked_issues;
use serde::{Deserialize, Serialize};

/// A verdict approves only at or above this confidence.
pub const APPROVE_MIN: f64 = 0.85;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    /// `blocker`, `major`, `minor` or `nit`.
    pub severity: String,
    pub file: String,
    #[serde(default)]
    pub line: Option<u32>,
    pub claim: String,
    pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Verdict {
    pub pr: u64,
    pub head_sha: String,
    /// `approve` or `request-changes`.
    pub verdict: String,
    pub confidence: f64,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub lessons_hit: Vec<String>,
    #[serde(default)]
    pub escalate_risk: bool,
}

/// What the verdict amounts to once the rules are applied.
#[derive(Debug, Clone, PartialEq)]
pub struct Effective {
    pub approve: bool,
    pub reasons: Vec<String>,
}

impl Verdict {
    /// Parse and validate JSON; the error names the offending field.
    pub fn parse(text: &str) -> Result<Verdict, String> {
        let v: Verdict = serde_json::from_str(text).map_err(|e| e.to_string())?;
        v.validate()?;
        Ok(v)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.head_sha.trim().is_empty() {
            return Err("head_sha is empty".into());
        }
        if !["approve", "request-changes"].contains(&self.verdict.as_str()) {
            return Err(format!(
                "verdict {:?} must be \"approve\" or \"request-changes\"",
                self.verdict
            ));
        }
        if !(0.0..=1.0).contains(&self.confidence) {
            return Err(format!(
                "confidence {} must be between 0 and 1",
                self.confidence
            ));
        }
        for (i, f) in self.findings.iter().enumerate() {
            if !["blocker", "major", "minor", "nit"].contains(&f.severity.as_str()) {
                return Err(format!(
                    "findings[{i}].severity {:?} must be blocker, major, minor or nit",
                    f.severity
                ));
            }
        }
        Ok(())
    }

    pub fn count(&self, severity: &str) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    }

    /// The status description, for example `approve 0.92, 0 blocker, 1 major`.
    pub fn summary(&self) -> String {
        format!(
            "{} {:.2}, {} blocker, {} major",
            self.verdict,
            self.confidence,
            self.count("blocker"),
            self.count("major")
        )
    }
}

/// Approve only with verdict `approve`, confidence at least [`APPROVE_MIN`] and no blocker.
pub fn effective(v: &Verdict) -> Effective {
    let mut reasons = Vec::new();
    if v.verdict != "approve" {
        reasons.push(format!("the reviewer said {}", v.verdict));
    }
    if v.confidence < APPROVE_MIN {
        reasons.push(format!(
            "confidence {:.2} is below {APPROVE_MIN}",
            v.confidence
        ));
    }
    let blockers = v.count("blocker");
    if blockers > 0 {
        reasons.push(format!("{blockers} blocker finding(s)"));
    }
    Effective {
        approve: reasons.is_empty(),
        reasons,
    }
}

/// The first issue a PR body links (see [`linked_issues`]).
pub fn closes_issue(body: &str) -> Option<u64> {
    linked_issues(body).first().copied()
}

/// Marks a PR comment that carries a verdict for `bosun verdict apply` (spec §7).
pub const COMMENT_MARKER: &str = "<!-- bosun:verdict -->";

const SESSION_TAG: &str = "<!-- bosun:reviewer ";

/// The PR comment `bosun verdict comment` posts: a summary, the reviewer session (for the
/// record; the session writes it, so it is not proof), the marker, the JSON.
pub fn comment_body(v: &Verdict, session: &str) -> String {
    let json = serde_json::to_string(v).unwrap_or_default();
    format!(
        "Review verdict for {}: {}.\n\n{SESSION_TAG}{session} -->\n{COMMENT_MARKER}\n```json\n{json}\n```",
        v.head_sha,
        v.summary()
    )
}

/// The reviewer session a verdict comment names, if any.
pub fn comment_session(body: &str) -> Option<&str> {
    let (_, rest) = body.split_once(SESSION_TAG)?;
    let (s, _) = rest.split_once(" -->")?;
    (!s.is_empty() && !s.contains(char::is_whitespace)).then_some(s)
}

/// The verdict JSON of a verdict comment: the text after the marker, without a code fence.
pub fn comment_json(body: &str) -> Option<&str> {
    let (_, rest) = body.split_once(COMMENT_MARKER)?;
    let rest = rest.trim();
    let rest = rest.strip_prefix("```json").unwrap_or(rest);
    Some(rest.strip_suffix("```").unwrap_or(rest).trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(verdict: &str, conf: f64, sev: &[&str]) -> Verdict {
        Verdict {
            pr: 1,
            head_sha: "abc".into(),
            verdict: verdict.into(),
            confidence: conf,
            findings: sev
                .iter()
                .map(|s| Finding {
                    severity: s.to_string(),
                    file: "f".into(),
                    line: None,
                    claim: "c".into(),
                    evidence: "e".into(),
                })
                .collect(),
            lessons_hit: vec![],
            escalate_risk: false,
        }
    }

    #[test]
    fn threshold_is_inclusive() {
        assert!(!effective(&v("approve", 0.84, &[])).approve);
        assert!(effective(&v("approve", 0.85, &[])).approve);
        assert!(!effective(&v("approve", 0.99, &["blocker"])).approve);
        assert!(effective(&v("approve", 0.9, &["major", "nit"])).approve);
        assert!(!effective(&v("request-changes", 0.99, &[])).approve);
    }

    #[test]
    fn summary_matches_the_spec_style() {
        assert_eq!(
            v("approve", 0.92, &["major"]).summary(),
            "approve 0.92, 0 blocker, 1 major"
        );
    }

    #[test]
    fn validation_names_the_field() {
        let bad = r#"{"pr":1,"head_sha":"a","verdict":"approve","confidence":0.9,"extra":1}"#;
        assert!(Verdict::parse(bad).unwrap_err().contains("extra"));
        let bad = r#"{"pr":1,"head_sha":"a","verdict":"approve"}"#;
        assert!(Verdict::parse(bad).unwrap_err().contains("confidence"));
        let bad = r#"{"pr":1,"head_sha":"a","verdict":"approve","confidence":1.5}"#;
        assert!(Verdict::parse(bad).unwrap_err().contains("confidence"));
        let bad = r#"{"pr":1,"head_sha":"a","verdict":"meh","confidence":0.5}"#;
        assert!(Verdict::parse(bad).unwrap_err().contains("verdict"));
        let bad = r#"{"pr":1,"head_sha":"a","verdict":"approve","confidence":0.9,
            "findings":[{"severity":"huge","file":"f","claim":"c","evidence":"e"}]}"#;
        assert!(
            Verdict::parse(bad)
                .unwrap_err()
                .contains("findings[0].severity")
        );
    }

    #[test]
    fn closes_is_found() {
        assert_eq!(closes_issue("Closes #12\n\nx"), Some(12));
        assert_eq!(closes_issue("this fixes #7."), Some(7));
        assert_eq!(closes_issue("no link"), None);
    }

    #[test]
    fn a_verdict_comment_round_trips() {
        let v = Verdict::parse(r#"{"pr":3,"head_sha":"abc","verdict":"approve","confidence":0.9}"#)
            .unwrap();
        let body = comment_body(&v, "session_r");
        assert!(body.contains(COMMENT_MARKER));
        assert_eq!(comment_session(&body), Some("session_r"));
        assert_eq!(comment_session("no tag"), None);
        assert_eq!(Verdict::parse(comment_json(&body).unwrap()).unwrap(), v);
        assert_eq!(comment_json("no marker"), None);
    }
}
