//! Task states and their labels (spec §4). Only bosun commands change a state label.

use std::fmt;
use std::str::FromStr;

pub const STATE_PREFIX: &str = "bosun:state/";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum State {
    Planned,
    Ready,
    Claimed,
    Building,
    Review,
    Fixing,
    Queued,
    Merged,
    Blocked,
    NeedsHuman,
}

impl State {
    pub const ALL: [State; 10] = [
        State::Planned,
        State::Ready,
        State::Claimed,
        State::Building,
        State::Review,
        State::Fixing,
        State::Queued,
        State::Merged,
        State::Blocked,
        State::NeedsHuman,
    ];

    pub fn name(self) -> &'static str {
        match self {
            State::Planned => "planned",
            State::Ready => "ready",
            State::Claimed => "claimed",
            State::Building => "building",
            State::Review => "review",
            State::Fixing => "fixing",
            State::Queued => "queued",
            State::Merged => "merged",
            State::Blocked => "blocked",
            State::NeedsHuman => "needs-human",
        }
    }

    /// The GitHub label, for example `bosun:state/ready`.
    pub fn label(self) -> String {
        format!("{STATE_PREFIX}{}", self.name())
    }

    /// A task in one of these states holds (or held) a lease.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            State::Claimed | State::Building | State::Review | State::Fixing | State::Queued
        )
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for State {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        State::ALL
            .into_iter()
            .find(|st| st.name() == s)
            .ok_or_else(|| format!("unknown state `{s}`"))
    }
}

/// The state named by an issue's labels; `None` when it has no state label. With several
/// state labels (a hand edit), the first in `State::ALL` order wins.
pub fn state_of<S: AsRef<str>>(labels: &[S]) -> Option<State> {
    State::ALL
        .into_iter()
        .find(|st| labels.iter().any(|l| l.as_ref() == st.label()))
}

/// The labels to remove when an issue moves to `to`: every other state label it carries.
pub fn labels_to_remove<S: AsRef<str>>(labels: &[S], to: State) -> Vec<String> {
    labels
        .iter()
        .map(|l| l.as_ref())
        .filter(|l| l.starts_with(STATE_PREFIX) && *l != to.label())
        .map(str::to_string)
        .collect()
}

/// The spec §4 table, plus the ways back out that release, reap and the owner use:
/// an active task goes back to ready (release, reap), and blocked or needs-human go back to
/// ready or review once the cause is gone.
pub fn transition_allowed(from: State, to: State) -> bool {
    use State::*;
    if from == to || matches!(to, Blocked | NeedsHuman) {
        return from != Merged;
    }
    matches!(
        (from, to),
        (Planned, Ready)
            | (Ready, Claimed)
            | (Claimed, Building)
            | (Claimed | Building | Fixing, Review)
            | (Review, Fixing)
            | (Review, Queued)
            | (Building | Review | Fixing | Queued, Merged)
            | (Queued, Fixing)
            | (Claimed | Building | Review | Fixing | Queued, Ready)
            | (Blocked | NeedsHuman, Ready | Review)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use State::*;

    #[test]
    fn names_round_trip() {
        for st in State::ALL {
            assert_eq!(st.name().parse::<State>().unwrap(), st);
        }
        assert_eq!(NeedsHuman.label(), "bosun:state/needs-human");
        assert!("nope".parse::<State>().is_err());
    }

    #[test]
    fn spec_table_is_allowed() {
        for (a, b) in [
            (Planned, Ready),
            (Ready, Claimed),
            (Claimed, Building),
            (Building, Review),
            (Review, Fixing),
            (Fixing, Review),
            (Review, Queued),
            (Queued, Merged),
            (Building, Merged),
            (Review, Merged),
            (Fixing, Merged),
            (Ready, Blocked),
            (Building, NeedsHuman),
        ] {
            assert!(transition_allowed(a, b), "{a} -> {b}");
        }
    }

    #[test]
    fn shortcuts_are_refused() {
        for (a, b) in [
            (Planned, Claimed),
            (Ready, Merged),
            (Claimed, Merged),
            (Blocked, Merged),
            (Merged, Ready),
            (Merged, Blocked),
            (Planned, Review),
        ] {
            assert!(!transition_allowed(a, b), "{a} -> {b}");
        }
    }

    #[test]
    fn state_from_labels() {
        let labels = ["bug", "bosun:state/review", "bosun:area/x"];
        assert_eq!(state_of(&labels), Some(Review));
        assert_eq!(state_of(&["bug"]), None);
        assert_eq!(
            labels_to_remove(&labels, Fixing),
            vec!["bosun:state/review".to_string()]
        );
        assert!(labels_to_remove(&labels, Review).is_empty());
    }
}
