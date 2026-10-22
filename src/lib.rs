#![forbid(unsafe_code)]
//! bosun: orchestration helper for repos where agents write all the code. See `docs/spec.md`.

pub mod cli;
pub mod config;
pub mod core;
pub mod ctx;
pub mod gates;
pub mod github;
pub mod output;
pub mod time;

pub use cli::{Cli, Command};
pub use output::{BosunError, CmdResult, Exit, Next, Outcome, envelope};

/// Run one command. `main` prints the envelope and exits with its code.
pub fn run(cli: Cli) -> CmdResult {
    let dir = cli.dir.as_deref();
    if let Command::Gate(a) = &cli.command {
        return cli::gate::run(dir, a);
    }
    let task = seed_task(&cli.command);
    Err(BosunError::error(
        format!("`bosun {}` is not implemented yet", cli.command.name()),
        format!(
            "it lands in seed task {task}; see docs/superpowers/plans/2026-10-22-bosun-seed.md"
        ),
    ))
}

/// The seed plan task that implements a command not dispatched yet.
fn seed_task(command: &Command) -> u8 {
    match command {
        Command::Join(_) => 6,
        Command::Claim(_) => 6,
        Command::Heartbeat => 6,
        Command::Release(_) => 6,
        Command::Reap => 6,
        Command::Spawn(_) => 9,
        Command::Wave(_) => 9,
        Command::Wait(_) => 10,
        Command::Msg(_) => 10,
        Command::RiskCheck(_) => 13,
        Command::TestIntegrity(_) => 12,
        Command::ReviewPack(_) => 14,
        Command::Verdict(_) => 14,
        Command::Pr(_) => 15,
        Command::CiLog(_) => 15,
        Command::Status => 16,
        Command::Ask(_) => 13,
        Command::Lesson(_) => 16,
        Command::Hook(_) => 7,
        Command::Doctor => 17,
        Command::Budget(_) => 17,
        _ => 0,
    }
}
