#![forbid(unsafe_code)]
//! bosun: orchestration helper for repos where agents write all the code. See `docs/spec.md`.

pub mod cli;
pub mod output;

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
        Command::Status => 16,
        Command::Doctor => 17,
    }
}
