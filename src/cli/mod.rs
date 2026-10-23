//! Command-line interface: one subcommand per bosun command (spec §11 v0.1 list, plus msg and
//! budget record). Each command module returns a `CmdResult`.

use clap::{Args, Parser, Subcommand};

pub mod ask;
pub mod budget;
pub mod ci_log;
pub mod claim;
pub mod doctor;
pub mod heartbeat;
pub mod join;
pub mod lesson;
pub mod msg;
pub mod plan;
pub mod pr;
pub mod reap;
pub mod release;
pub mod review_pack;
pub mod risk_check;
pub mod spawn;
pub mod status;
pub mod verdict;
pub mod wait;
pub mod wave;
use std::path::PathBuf;

pub mod gate;
pub mod stop;
pub mod test_integrity;

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
    /// Run the 7 spawn checks, claim the epic, and return the create_session call.
    Spawn(SpawnArgs),
    /// Wave commands.
    #[command(subcommand)]
    Wave(WaveCmd),
    /// Block until a PR, an issue or a child session changes state (polls GitHub).
    Wait(WaitArgs),
    /// Return a send_message call to steer another session (at most 500 characters).
    Msg(MsgArgs),
    /// Run one gate of the stack's profile (fmt, lint, test, deps, ...).
    Gate(GateArgs),
    /// Judge a PR with the risk rules of its base branch.
    RiskCheck(PrArg),
    /// Refuse deleted tests, new skips and fewer assertions.
    TestIntegrity(IntegrityArgs),
    /// Build the input of a review: diff, task, UCs, lessons, REVIEW.md.
    ReviewPack(PrArg),
    /// Verdict commands.
    #[command(subcommand)]
    Verdict(VerdictCmd),
    /// Open or update the PR of a task; --auto-merge after all gates are green.
    Pr(PrArgs),
    /// Print the tail of the failed CI jobs of a PR.
    CiLog(CiLogArgs),
    /// Show sessions, leases, tasks by state, halt, main-red and budget.
    Status,
    /// Ask the owner a question on an issue (sets needs-human).
    Ask(AskArgs),
    /// Lesson commands.
    #[command(subcommand)]
    Lesson(LessonCmd),
    /// Claude Code hook entry points.
    #[command(subcommand)]
    Hook(HookCmd),
    /// Report each checklist step as ok or missing.
    Doctor,
    /// Budget commands.
    #[command(subcommand)]
    Budget(BudgetCmd),
    /// Plan commands.
    #[command(subcommand)]
    Plan(PlanCmd),
    /// Record a failed fix round; 3 equal failures or 0 changed lines block the task and escalate.
    Fail(FailArgs),
}

/// Top-level command names, in `--help` order.
pub const COMMANDS: [&str; 24] = [
    "join",
    "claim",
    "heartbeat",
    "release",
    "reap",
    "spawn",
    "wave",
    "wait",
    "msg",
    "gate",
    "risk-check",
    "test-integrity",
    "review-pack",
    "verdict",
    "pr",
    "ci-log",
    "status",
    "ask",
    "lesson",
    "hook",
    "doctor",
    "budget",
    "plan",
    "fail",
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
            Command::Spawn(_) => "spawn",
            Command::Wave(WaveCmd::Plan(_)) => "wave plan",
            Command::Wait(_) => "wait",
            Command::Msg(_) => "msg",
            Command::Gate(_) => "gate",
            Command::RiskCheck(_) => "risk-check",
            Command::TestIntegrity(_) => "test-integrity",
            Command::ReviewPack(_) => "review-pack",
            Command::Verdict(VerdictCmd::Post(_)) => "verdict post",
            Command::Verdict(VerdictCmd::Comment(_)) => "verdict comment",
            Command::Verdict(VerdictCmd::Apply(_)) => "verdict apply",
            Command::Pr(_) => "pr",
            Command::CiLog(_) => "ci-log",
            Command::Status => "status",
            Command::Ask(_) => "ask",
            Command::Lesson(LessonCmd::Add(_)) => "lesson add",
            Command::Lesson(LessonCmd::Hit(_)) => "lesson hit",
            Command::Hook(HookCmd::PreToolUse) => "hook pre-tool-use",
            Command::Hook(HookCmd::SessionStart) => "hook session-start",
            Command::Doctor => "doctor",
            Command::Budget(BudgetCmd::Record(_)) => "budget record",
            Command::Plan(PlanCmd::Check(_)) => "plan check",
            Command::Plan(PlanCmd::Import(_)) => "plan import",
            Command::Fail(_) => "fail",
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

#[derive(Debug, Args)]
pub struct SpawnArgs {
    /// The epic issue number.
    pub epic: u64,
    /// Model of the new session (never a [1m] model).
    #[arg(long)]
    pub model: Option<String>,
    /// File with the prompt of the new session.
    #[arg(long)]
    pub prompt_file: Option<PathBuf>,
    /// Register the child from the create_session result (with --from -).
    #[arg(long)]
    pub record: bool,
    /// Read a tool result from this file, or `-` for stdin.
    #[arg(long)]
    pub from: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum WaveCmd {
    /// The next set of tasks of an epic that can run in parallel.
    Plan(EpicArg),
}

#[derive(Debug, Args)]
pub struct EpicArg {
    /// The epic issue number.
    pub epic: u64,
}

#[derive(Debug, Args)]
pub struct WaitArgs {
    #[arg(long, group = "target")]
    pub pr: Option<u64>,
    #[arg(long, group = "target")]
    pub issue: Option<u64>,
    #[arg(long, group = "target", value_name = "SESSION")]
    pub child: Option<String>,
    /// Give up after this many seconds (exit 0, changed: false).
    #[arg(long, default_value_t = 1800)]
    pub timeout: u64,
}

#[derive(Debug, Args)]
pub struct MsgArgs {
    pub session: String,
    pub text: String,
    /// The issue the message is about.
    #[arg(long)]
    pub issue: Option<u64>,
}

#[derive(Debug, Args)]
pub struct GateArgs {
    /// fmt, lint, types, test, deps, mutants, snapshots, test-integrity, main-open, ...
    pub name: String,
    /// Base ref for the diff (default: the PR base or origin/main).
    #[arg(long)]
    pub base: Option<String>,
    /// Run on all packages (merge queue).
    #[arg(long)]
    pub all: bool,
}

#[derive(Debug, Args)]
pub struct PrArg {
    /// The PR number.
    pub pr: u64,
}

#[derive(Debug, Args)]
pub struct IntegrityArgs {
    #[arg(long)]
    pub base: Option<String>,
    #[arg(long)]
    pub pr: Option<u64>,
}

#[derive(Debug, Subcommand)]
pub enum VerdictCmd {
    /// Post a reviewer verdict as the review-verdict status.
    Post(FromArg),
    /// Post the verdict as a PR comment; the verdict workflow applies it (no status write).
    Comment(FromArg),
    /// Check a verdict comment and post its review-verdict status (the verdict workflow).
    Apply(ApplyArg),
}

#[derive(Debug, Args)]
pub struct ApplyArg {
    /// The id of the PR comment that carries the verdict.
    pub comment_id: u64,
}

#[derive(Debug, Args)]
pub struct FromArg {
    /// Read the JSON from this file, or `-` for stdin.
    #[arg(long, default_value = "-")]
    pub from: String,
}

#[derive(Debug, Args)]
pub struct PrArgs {
    /// The task issue number.
    pub issue: u64,
    #[arg(long)]
    pub title: Option<String>,
    /// Turn on auto-merge once every required check is green (bosun never merges).
    #[arg(long)]
    pub auto_merge: bool,
}

#[derive(Debug, Args)]
pub struct CiLogArgs {
    pub pr: u64,
    #[arg(long, default_value_t = 80)]
    pub tail: usize,
}

#[derive(Debug, Args)]
pub struct AskArgs {
    pub issue: u64,
    pub question: String,
}

#[derive(Debug, Subcommand)]
pub enum LessonCmd {
    /// Add a lesson to docs/lessons.toml (exit 6 at the cap).
    Add(LessonAddArgs),
    /// Record a hit on a lesson.
    Hit(LessonHitArgs),
}

#[derive(Debug, Args)]
pub struct LessonAddArgs {
    pub text: String,
    #[arg(long = "scope", value_name = "GLOB")]
    pub scope: Vec<String>,
    #[arg(long)]
    pub why: Option<String>,
    #[arg(long = "evidence", value_name = "REF")]
    pub evidence: Vec<String>,
}

#[derive(Debug, Args)]
pub struct LessonHitArgs {
    pub id: String,
    #[arg(long)]
    pub issue: Option<u64>,
}

#[derive(Debug, Subcommand)]
pub enum HookCmd {
    /// PreToolUse: edit-lock, label-lock, merge-lock.
    PreToolUse,
    /// SessionStart: join and print the session context.
    SessionStart,
}

#[derive(Debug, Subcommand)]
pub enum BudgetCmd {
    /// Record the spend of a task (claude-sessions or generic provider).
    Record(BudgetRecordArgs),
}

#[derive(Debug, Args)]
pub struct BudgetRecordArgs {
    pub issue: u64,
    #[arg(long, value_parser = ["claude-sessions", "generic"])]
    pub provider: String,
    #[arg(long)]
    pub from: Option<String>,
    #[arg(long)]
    pub turns: Option<u64>,
    #[arg(long)]
    pub model: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum PlanCmd {
    /// Parse a plan file and report every error with its line and fix (no GitHub).
    Check(PlanCheckArgs),
    /// Create the epics, plan tasks and tasks of a plan that do not exist yet.
    Import(PlanImportArgs),
}

#[derive(Debug, Args)]
pub struct PlanCheckArgs {
    /// The plan file, for example plans/bosun.md.
    pub file: String,
}

#[derive(Debug, Args)]
pub struct PlanImportArgs {
    /// The plan file, for example plans/bosun.md.
    pub file: String,
    /// Show what would be created; write nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// Import only this item (repeatable); default all.
    #[arg(long = "item", value_name = "ID")]
    pub item: Vec<String>,
}

#[derive(Debug, Args)]
pub struct FailArgs {
    /// The task issue number.
    pub issue: u64,
    /// A short stable text for the failure, for example `E0308 in src/x.rs`.
    #[arg(long)]
    pub signature: String,
    /// Lines the fix round changed; 0 blocks the task at once.
    #[arg(long)]
    pub changed_lines: Option<u64>,
    /// The model tier in use (default: the newest escalation, else the build model).
    #[arg(long)]
    pub model: Option<String>,
}
