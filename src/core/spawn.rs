//! The 7 spawn checks of spec §5, in order. Check 7 (claim, then create_session) is the
//! command's own work; this module decides checks 1-6.

use crate::output::Exit;

/// Everything the checks need, gathered by the command.
#[derive(Debug, Clone, PartialEq)]
pub struct SpawnInput {
    pub main_red: bool,
    pub process_red: bool,
    /// Mainfix work skips checks 1 and 2.
    pub mainfix: bool,
    /// A process-fix epic is exempt from process-red, as mainfix is from main-red.
    pub process_fix: bool,
    pub spent_today_usd: f64,
    pub daily_usd: f64,
    /// Depth of the spawning session (root is 1).
    pub parent_depth: u32,
    pub max_depth: u32,
    /// Live sessions of all origins.
    pub live_sessions: u32,
    pub max_live_sessions: u32,
    pub children: u32,
    pub max_children: u32,
    /// A live lease or area owner that conflicts with the epic: (what, who).
    pub conflict: Option<String>,
}

/// A failed check: its number (1-6), name, exit code, message and fix.
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub check: u8,
    pub name: &'static str,
    pub exit: Exit,
    pub message: String,
    pub fix: String,
}

/// Share of the daily budget at which new work stops (spec §9).
pub const BUDGET_STOP: f64 = 0.8;

/// Run checks 1-6 in order; the first failure is the refusal.
pub fn check(i: &SpawnInput) -> Result<(), Refusal> {
    let refuse = |check, name, exit, message: String, fix: &str| {
        Err(Refusal {
            check,
            name,
            exit,
            message,
            fix: fix.to_string(),
        })
    };
    if i.main_red && !i.mainfix {
        return refuse(
            1,
            "main-red",
            Exit::Halted,
            "main is red; only mainfix work may start".into(),
            "wait for the main-red issue to close (`bosun wait --issue <n>`), or work on the mainfix task",
        );
    }
    if i.process_red && !i.mainfix && !i.process_fix {
        return refuse(
            1,
            "process-red",
            Exit::Halted,
            "process is red; only mainfix and process-fix work may start".into(),
            "close the process-red issue by merging its process-fix, or work on a process-fix task",
        );
    }
    if !i.mainfix && i.daily_usd > 0.0 && i.spent_today_usd >= BUDGET_STOP * i.daily_usd {
        return refuse(
            2,
            "budget",
            Exit::Limit,
            format!(
                "spent ${:.2} of ${:.2} today (at or over {:.0} %)",
                i.spent_today_usd,
                i.daily_usd,
                BUDGET_STOP * 100.0
            ),
            "wait for the day to reset at 00:00 UTC, or ask the owner to raise budget.daily_usd (`bosun ask`)",
        );
    }
    let depth = i.parent_depth + 1;
    if depth > i.max_depth {
        return refuse(
            3,
            "depth",
            Exit::Limit,
            format!(
                "the new session would have depth {depth}, over max_depth {}",
                i.max_depth
            ),
            "run the epic's tasks as wave agents in this session (`bosun wave plan`) instead of spawning",
        );
    }
    if i.live_sessions >= i.max_live_sessions {
        return refuse(
            4,
            "live-sessions",
            Exit::Limit,
            format!(
                "{} live sessions, max_live_sessions is {}",
                i.live_sessions, i.max_live_sessions
            ),
            "wait for a session to finish (`bosun wait --child <id>`), or run `bosun reap` to drop dead ones",
        );
    }
    if i.children >= i.max_children {
        return refuse(
            5,
            "children",
            Exit::Limit,
            format!(
                "this session has {} children, max_children is {}",
                i.children, i.max_children
            ),
            "wait for a child to post its result (`bosun wait --child <id>`)",
        );
    }
    if let Some(what) = &i.conflict {
        return refuse(
            6,
            "conflict",
            Exit::Conflict,
            format!("the epic conflicts with {what}"),
            "pick another epic, or wait for that lease to end",
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_input() -> SpawnInput {
        SpawnInput {
            main_red: false,
            process_red: false,
            process_fix: false,
            mainfix: false,
            spent_today_usd: 10.0,
            daily_usd: 150.0,
            parent_depth: 1,
            max_depth: 2,
            live_sessions: 1,
            max_live_sessions: 4,
            children: 0,
            max_children: 3,
            conflict: None,
        }
    }

    fn refused(i: SpawnInput) -> (u8, Exit) {
        let r = check(&i).unwrap_err();
        assert!(!r.fix.is_empty());
        (r.check, r.exit)
    }

    #[test]
    fn all_clear() {
        assert_eq!(check(&ok_input()), Ok(()));
    }

    #[test]
    fn one_refusal_per_check_in_order() {
        let mut i = ok_input();
        i.main_red = true;
        assert_eq!(refused(i), (1, Exit::Halted));
        let mut i = ok_input();
        i.process_red = true;
        assert_eq!(refused(i), (1, Exit::Halted));
        let mut i = ok_input();
        i.spent_today_usd = 120.0; // exactly 80 %
        assert_eq!(refused(i), (2, Exit::Limit));
        let mut i = ok_input();
        i.parent_depth = 2;
        assert_eq!(refused(i), (3, Exit::Limit));
        let mut i = ok_input();
        i.live_sessions = 4;
        assert_eq!(refused(i), (4, Exit::Limit));
        let mut i = ok_input();
        i.children = 3;
        assert_eq!(refused(i), (5, Exit::Limit));
        let mut i = ok_input();
        i.conflict = Some("a lease".into());
        assert_eq!(refused(i), (6, Exit::Conflict));
    }

    #[test]
    fn earlier_check_wins() {
        let mut i = ok_input();
        i.main_red = true;
        i.children = 9;
        assert_eq!(refused(i).0, 1);
    }

    #[test]
    fn process_red_exempts_mainfix_and_process_fix() {
        let mut i = ok_input();
        i.process_red = true;
        i.process_fix = true;
        assert_eq!(check(&i), Ok(()));
        i.process_fix = false;
        i.mainfix = true;
        assert_eq!(check(&i), Ok(()));
        // process-fix work does not skip main-red or the budget.
        let mut i = ok_input();
        i.process_fix = true;
        i.main_red = true;
        assert_eq!(refused(i), (1, Exit::Halted));
        let mut i = ok_input();
        i.process_fix = true;
        i.spent_today_usd = 120.0;
        assert_eq!(refused(i), (2, Exit::Limit));
    }

    #[test]
    fn mainfix_skips_main_red_and_budget_only() {
        let mut i = ok_input();
        i.mainfix = true;
        i.main_red = true;
        i.spent_today_usd = 1000.0;
        assert_eq!(check(&i), Ok(()));
        i.live_sessions = 4;
        assert_eq!(refused(i).0, 4);
    }
}
