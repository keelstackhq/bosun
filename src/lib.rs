#![forbid(unsafe_code)]
//! bosun: orchestration helper for repos where agents write all the code. See `docs/spec.md`.

pub mod cli;
pub mod config;
pub mod ctx;
pub mod gates;
pub mod output;
pub mod time;

pub use cli::{Cli, Command};
pub use output::{BosunError, CmdResult, Exit, Next, Outcome, envelope};

/// Run one command. `main` prints the envelope and exits with its code.
pub fn run(cli: Cli) -> CmdResult {
    let task = seed_task(&cli.command);
    Err(BosunError::error(
        format!("`bosun {}` is not implemented yet", cli.command.name()),
        format!(
            "it lands in seed task {task}; see docs/superpowers/plans/2026-10-22-bosun-seed.md"
        ),
    ))
}

/// The seed plan task that implements a command.
fn seed_task(command: &Command) -> u8 {
    match command {
        Command::Join(_)
        | Command::Claim(_)
        | Command::Heartbeat
        | Command::Release(_)
        | Command::Reap => 6,
        Command::Hook(_) => 7,
        Command::Spawn(_) | Command::Wave(_) => 9,
        Command::Wait(_) | Command::Msg(_) => 10,
        Command::Gate(_) => 11,
        Command::TestIntegrity(_) => 12,
        Command::RiskCheck(_) | Command::Ask(_) => 13,
        Command::ReviewPack(_) | Command::Verdict(_) => 14,
        Command::Pr(_) | Command::CiLog(_) => 15,
        Command::Lesson(_) | Command::Status => 16,
        Command::Doctor | Command::Budget(_) => 17,
    }
}
