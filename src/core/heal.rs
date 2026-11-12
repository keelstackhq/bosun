//! `bosun heal` logic (plan PR 3a): what to do about one open PR whose head fails `ci-ok`.
//! Pure: the command reads the PR, its task and the brake.

use super::mode::{HealMain, HealPr};
use super::stop::Stop;

/// Heal rounds per PR; one more red head after that goes to the digest as needs-human.
pub const MAX_ROUNDS: usize = 2;

const MARK: &str = "<!-- bosun:heal head=";

/// The PR comment marker of one heal round (one per head sha).
pub fn marker(head: &str, round: usize, action: &str) -> String {
    format!("{MARK}{head} round={round} action={action} -->")
}

/// The heads already healed, in order, once each, from the bodies of trusted PR comments.
pub fn healed_heads<'a>(bodies: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in bodies.into_iter().flat_map(str::lines) {
        let head = line
            .trim()
            .strip_prefix(MARK)
            .and_then(|r| r.split(' ').next());
        if let Some(h) = head.filter(|h| !h.is_empty() && !out.iter().any(|o| o == h)) {
            out.push(h.to_string());
        }
    }
    out
}

/// The knob after the brake: while it is on, heal at most notifies.
pub fn capped(knob: HealPr, brake_on: bool) -> HealPr {
    if brake_on {
        knob.min(HealPr::Notify)
    } else {
        knob
    }
}

/// `heal_main` after the brake: while it is on, a revert PR is opened but never auto-merged.
pub fn capped_main(k: HealMain, brake_on: bool) -> HealMain {
    k.min(if brake_on {
        HealMain::RevertPr
    } else {
        HealMain::RevertAuto
    })
}

/// The PR a red main points at: `heads` are the merged PRs, newest first, with whether the merge
/// commit is green on `ci-ok`. Only the one PR right before the first green is a safe revert.
pub fn since_green(heads: &[(u64, bool)]) -> Result<u64, String> {
    match heads.iter().position(|h| h.1) {
        Some(0) => Err("the newest merge is green".into()),
        Some(1) => Ok(heads[0].0),
        Some(n) => Err(format!(
            "{n} PRs merged since green: run `bosun main-triage --good <sha>`, then `bosun revert <pr>`"
        )),
        None => Err("no green merge in the window".into()),
    }
}

/// One red PR as `decide` sees it.
pub struct RedPr<'a> {
    pub knob: HealPr,
    pub head: &'a str,
    pub healed: &'a [String],
    /// The stop rule that applies to the task (`core::stop::evaluate`).
    pub stop: Option<&'a Stop>,
    /// The session of a live lease on the task.
    pub holder: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Skip(String),
    Notify(String),
    Spawn,
    NeedsHuman(String),
}

pub fn decide(p: &RedPr) -> Action {
    if p.knob == HealPr::Off {
        return Action::Skip("heal_pr is off".into());
    }
    if p.healed.iter().any(|h| h == p.head) {
        return Action::Skip("this head was healed already; wait for the next push".into());
    }
    if let Some(s) = p.stop {
        return Action::Skip(format!(
            "stop rule {} blocks the task ({})",
            s.rule, s.signature
        ));
    }
    if p.healed.len() >= MAX_ROUNDS {
        return Action::NeedsHuman(format!("{MAX_ROUNDS} heal rounds did not turn ci-ok green"));
    }
    match (p.holder, p.knob) {
        (Some(s), _) => Action::Notify(s.to_string()),
        (None, HealPr::Spawn) => Action::Spawn,
        _ => Action::Skip("no live lease holder to notify; heal_pr spawn starts a fixer".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red<'a>(knob: HealPr, healed: &'a [String], holder: Option<&'a str>) -> RedPr<'a> {
        RedPr {
            knob,
            head: "h2",
            healed,
            stop: None,
            holder,
        }
    }

    #[test]
    fn markers_round_trip_once_per_head() {
        let a = format!("x\n{}", marker("h1", 1, "notify"));
        let b = marker("h1", 1, "notify");
        let c = marker("h2", 2, "spawn");
        let forged = "<!-- bosun:heal head= round=1 -->";
        assert_eq!(healed_heads([a.as_str(), &b, &c, forged]), ["h1", "h2"]);
    }

    #[test]
    fn brake_caps_spawn_at_notify_only() {
        assert_eq!(capped(HealPr::Spawn, true), HealPr::Notify);
        assert_eq!(capped(HealPr::Notify, true), HealPr::Notify);
        assert_eq!(capped(HealPr::Off, true), HealPr::Off);
        assert_eq!(capped(HealPr::Spawn, false), HealPr::Spawn);
    }

    #[test]
    fn decide_order() {
        let none: Vec<String> = vec![];
        let one = vec!["h1".to_string()];
        let two = vec!["h0".to_string(), "h1".to_string()];
        let same = vec!["h2".to_string()];
        let n = |s: &str| Action::Notify(s.into());
        assert_eq!(decide(&red(HealPr::Notify, &none, Some("s"))), n("s"));
        assert_eq!(decide(&red(HealPr::Spawn, &one, Some("s"))), n("s"));
        assert_eq!(decide(&red(HealPr::Spawn, &one, None)), Action::Spawn);
        assert!(matches!(
            decide(&red(HealPr::Notify, &none, None)),
            Action::Skip(_)
        ));
        assert!(matches!(
            decide(&red(HealPr::Off, &none, Some("s"))),
            Action::Skip(_)
        ));
        assert!(matches!(
            decide(&red(HealPr::Spawn, &same, None)),
            Action::Skip(_)
        ));
        assert!(matches!(
            decide(&red(HealPr::Spawn, &two, None)),
            Action::NeedsHuman(_)
        ));
        let stop = Stop {
            rule: "same-failure",
            signature: "sig".into(),
            streak: 3,
        };
        let mut p = red(HealPr::Spawn, &two, None);
        p.stop = Some(&stop);
        assert!(matches!(decide(&p), Action::Skip(w) if w.contains("same-failure")));
    }
}
