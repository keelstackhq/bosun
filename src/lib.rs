#![forbid(unsafe_code)]
//! bosun: orchestration helper for repos where agents write all the code. See `docs/spec.md`.

pub mod app;
pub mod cli;
pub mod config;
pub mod core;
pub mod ctx;
pub mod gates;
pub mod github;
pub mod hooks;
pub mod output;
pub mod registry;
pub mod time;

pub use cli::{Cli, Command};
pub use output::{BosunError, CmdResult, Exit, Next, Outcome, envelope};

/// Run one command. `main` prints the envelope and exits with its code.
pub fn run(cli: Cli) -> CmdResult {
    let dir = cli.dir.as_deref();
    match &cli.command {
        Command::Join(a) => return cli::join::run(dir, a),
        Command::Claim(a) => return cli::claim::run(dir, a),
        Command::Release(a) => return cli::release::run(dir, a),
        Command::Spawn(a) => return cli::spawn::run(dir, a),
        Command::Wave(a) => return cli::wave::run(dir, a),
        Command::Wait(a) => return cli::wait::run(dir, a),
        Command::Msg(a) => return cli::msg::run(dir, a),
        Command::Reap => return cli::reap::run(dir),
        Command::Heartbeat => return cli::heartbeat::run(dir),
        Command::Gate(a) => return cli::gate::run(dir, a),
        Command::TestIntegrity(a) => return cli::test_integrity::run(dir, a),
        Command::Hook(h) => return hooks::dispatch(dir, h),
        Command::RiskCheck(a) => return cli::risk_check::run(dir, a),
        Command::Ask(a) => return cli::ask::run(dir, a),
        Command::Verdict(a) => return cli::verdict::run(dir, a),
        Command::Pr(a) => return cli::pr::run(dir, a),
        Command::CiLog(a) => return cli::ci_log::run(dir, a),
        Command::ReviewPack(a) => return cli::review_pack::run(dir, a),
        _ => {}
    }
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
