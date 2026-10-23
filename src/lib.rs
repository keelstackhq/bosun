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
pub mod sessions;
pub mod time;

pub use cli::{Cli, Command};
pub use output::{BosunError, CmdResult, Exit, Next, Outcome, envelope};

/// Run one command. `main` prints the envelope and exits with its code.
pub fn run(cli: Cli) -> CmdResult {
    let dir = cli.dir.as_deref();
    match &cli.command {
        Command::Join(a) => cli::join::run(dir, a),
        Command::Claim(a) => cli::claim::run(dir, a),
        Command::Heartbeat => cli::heartbeat::run(dir),
        Command::Release(a) => cli::release::run(dir, a),
        Command::Reap => cli::reap::run(dir),
        Command::Spawn(a) => cli::spawn::run(dir, a),
        Command::Wave(a) => cli::wave::run(dir, a),
        Command::Wait(a) => cli::wait::run(dir, a),
        Command::Msg(a) => cli::msg::run(dir, a),
        Command::Gate(a) => cli::gate::run(dir, a),
        Command::RiskCheck(a) => cli::risk_check::run(dir, a),
        Command::TestIntegrity(a) => cli::test_integrity::run(dir, a),
        Command::ReviewPack(a) => cli::review_pack::run(dir, a),
        Command::Verdict(a) => cli::verdict::run(dir, a),
        Command::Pr(a) => cli::pr::run(dir, a),
        Command::CiLog(a) => cli::ci_log::run(dir, a),
        Command::Status => cli::status::run(dir),
        Command::Ask(a) => cli::ask::run(dir, a),
        Command::Lesson(a) => cli::lesson::run(dir, a),
        Command::Hook(a) => hooks::dispatch(dir, a),
        Command::Doctor => cli::doctor::run(dir),
        Command::Budget(a) => cli::budget::run(dir, a),
        Command::Fail(a) => cli::stop::run(dir, a),
        Command::MainTriage(a) => cli::main_triage::run(dir, a),
    }
}
