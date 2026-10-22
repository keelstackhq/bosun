//! Command-line interface: one subcommand per bosun command (spec §11 v0.1 list, plus msg and
//! budget record). Each command module returns a `CmdResult`.

use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "bosun",
    version,
    about = "Orchestration helper for repos where agents write all the code",
    long_about = "Orchestration helper for repos where agents write all the code.\n\n\
        stdout is always one JSON object {version, command, ok, exit_code, data, error, next}.\n\
        Exit codes: 0 ok, 1 error, 2 usage, 3 conflict, 4 halted, 5 needs-human, 6 limit."
)]
pub struct Cli {
    /// Run as if bosun was started in this directory.
    #[arg(short = 'C', global = true, value_name = "DIR")]
    pub dir: Option<PathBuf>,
    /// Accepted for symmetry with splice, hoist and berth; output is always JSON.
    #[arg(long, global = true)]
    pub json: bool,
    /// Log API calls and child commands to stderr.
    #[arg(short, long, global = true)]
    pub verbose: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Register this session in the registry (origin, area, parent, depth).
    Join(JoinArgs),
    /// Claim a ready task: write a lease for its touch paths (exit 3 on overlap).
    Claim(ClaimArgs),
    /// Extend every live lease and the registry record of this session.
    Heartbeat,
    /// End a lease in the normal way and move the task to a new state.
    Release(ReleaseArgs),
    /// Release expired leases and set their tasks back to ready.
    Reap,
    /// Show sessions, leases, tasks by state, halt, main-red and budget.
    Status,
    /// Claude Code hook entry points.
    #[command(subcommand)]
    Hook(HookCmd),
    /// Report each checklist step as ok or missing.
    Doctor,
}

/// Top-level command names, in `--help` order.
pub const COMMANDS: [&str; 8] = [
    "join",
    "claim",
    "heartbeat",
    "release",
    "reap",
    "status",
    "hook",
    "doctor",
];

impl Command {
    /// The name in the envelope; subcommand groups include the subcommand (`wave plan`).
    pub fn name(&self) -> &'static str {
        match self {
            Command::Join(_) => "join",
            Command::Claim(_) => "claim",
            Command::Heartbeat => "heartbeat",
            Command::Release(_) => "release",
            Command::Reap => "reap",
            Command::Status => "status",
            Command::Hook(HookCmd::PreToolUse) => "hook pre-tool-use",
            Command::Hook(HookCmd::SessionStart) => "hook session-start",
            Command::Doctor => "doctor",
        }
    }
}

#[derive(Debug, Args)]
pub struct JoinArgs {
    /// spawned, slack, local or routine (default from BOSUN_ORIGIN).
    #[arg(long)]
    pub origin: Option<String>,
    /// The area this session orchestrates.
    #[arg(long)]
    pub area: Option<String>,
    /// The parent session id.
    #[arg(long)]
    pub parent: Option<String>,
    /// Depth of this session (root is 1).
    #[arg(long)]
    pub depth: Option<u32>,
}

#[derive(Debug, Args)]
pub struct ClaimArgs {
    /// The task issue number.
    pub issue: u64,
    /// Touch path globs; default from the issue metadata.
    #[arg(long = "touch", value_name = "GLOB")]
    pub touch: Vec<String>,
}

#[derive(Debug, Args)]
pub struct ReleaseArgs {
    /// The task issue number.
    pub issue: u64,
    /// The state after the release.
    #[arg(long = "to", default_value = "ready", value_parser = ["ready", "blocked", "review"])]
    pub to: String,
}

#[derive(Debug, Subcommand)]
pub enum HookCmd {
    /// PreToolUse: edit-lock, label-lock, merge-lock.
    PreToolUse,
    /// SessionStart: join and print the session context.
    SessionStart,
}
