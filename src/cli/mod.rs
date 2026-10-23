//! Command-line interface: one subcommand per bosun command (spec §11 v0.1 list, plus msg and
//! budget record). Each command module returns a `CmdResult`.

use clap::{Args, Parser, Subcommand};

pub mod ask;
pub mod budget;
pub mod ci_log;
pub mod claim;
pub mod doctor;
pub mod heartbeat;
pub mod init;
pub mod intake;
pub mod join;
pub mod lesson;
pub mod lesson_triage;
pub mod main_triage;
pub mod msg;
pub mod plan;
pub mod pr;
pub mod progress;
pub mod reap;
pub mod release;
pub mod result;
pub mod resume;
pub mod revert;
pub mod review_pack;
pub mod risk_check;
pub mod spawn;
pub mod status;
pub mod verdict;
pub mod wait;
pub mod wave;
use std::path::PathBuf;

pub mod gate;
pub mod main_health;
pub mod review_ci;
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
    Doctor(DoctorArgs),
    /// Budget commands.
    #[command(subcommand)]
    Budget(BudgetCmd),
    /// Plan commands.
    #[command(subcommand)]
    Plan(PlanCmd),
    /// Record a failed fix round; 3 equal failures or 0 changed lines block the task and escalate.
    Fail(FailArgs),
    /// After a container restart: renew this session's leases and list the PRs to continue.
    Resume,
    /// Cost per merged task over the last N days (read-only).
    Progress(ProgressArgs),
    /// Post this session's result on its epic, end its lease and leave the registry.
    Result(ResultArgs),
    /// Bisect the merged PRs between a good and a bad rev; name the first bad one.
    MainTriage(MainTriageArgs),
    /// Set up this repo for bosun: add the splice channel and open the setup PR.
    Init(InitArgs),
    /// Open a kind/mainfix PR that reverts a merged PR.
    Revert(RevertArgs),
    /// Open a kind/plan issue that asks for a plan amendment (the only way new work enters).
    Intake(IntakeArgs),
    /// Open or close the main-red issue from a CI result; re-judge the open PRs (spec §9).
    MainHealth(MainHealthArgs),
    /// Review mode b: ask the Anthropic API for the verdict of a PR and record it (no session).
    ReviewCi(PrArg),
}

/// Top-level command names, in `--help` order.
pub const COMMANDS: &[&str] = &[
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
    "resume",
    "progress",
    "result",
    "main-triage",
    "init",
    "revert",
    "intake",
    "main-health",
    "review-ci",
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
            Command::Lesson(LessonCmd::Capture(_)) => "lesson capture",
            Command::Lesson(LessonCmd::Triage(_)) => "lesson triage",
            Command::Hook(HookCmd::PreToolUse) => "hook pre-tool-use",
            Command::Hook(HookCmd::SessionStart) => "hook session-start",
            Command::Doctor(_) => "doctor",
            Command::Budget(BudgetCmd::Record(_)) => "budget record",
            Command::Plan(PlanCmd::Check(_)) => "plan check",
            Command::Plan(PlanCmd::Import(_)) => "plan import",
            Command::Plan(PlanCmd::Amend(_)) => "plan amend",
            Command::Fail(_) => "fail",
            Command::Resume => "resume",
            Command::Progress(_) => "progress",
            Command::Budget(BudgetCmd::Check) => "budget check",
            Command::Result(_) => "result",
            Command::MainTriage(_) => "main-triage",
            Command::Init(_) => "init",
            Command::Revert(_) => "revert",
            Command::Intake(_) => "intake",
            Command::MainHealth(_) => "main-health",
            Command::ReviewCi(_) => "review-ci",
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
pub struct ResultArgs {
    /// The epic issue number.
    pub epic: u64,
    /// How the work ended.
    #[arg(long, value_parser = ["merged", "blocked", "needs-human"])]
    pub outcome: String,
    /// The PR of the work.
    #[arg(long)]
    pub pr: Option<u64>,
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
    /// The PR number, for `main-open` (default: env `BOSUN_PR`).
    #[arg(long)]
    pub pr: Option<u64>,
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
pub struct DoctorArgs {
    /// Also run `hoist check` (120 s limit, BOSUN_HOIST_TIMEOUT_SECS): drift fails step 3 (github
    /// settings and checks), 5 (release and environments) or 10 (npm and crates first publish);
    /// unknown items and verify notes are listed in data.hoist
    #[arg(long)]
    pub remote: bool,
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
    /// Open a lesson issue for a failure, or add a hit to the one with the same signature.
    Capture(LessonCaptureArgs),
    /// Triage a lesson issue: blocking, mechanizable or advice (spec §6).
    Triage(LessonTriageArgs),
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
    /// File the lesson on keelstackhq/bosun (it is about bosun itself); no local change.
    #[arg(long)]
    pub upstream: bool,
}

#[derive(Debug, Args)]
pub struct LessonHitArgs {
    pub id: String,
    #[arg(long)]
    pub issue: Option<u64>,
}

#[derive(Debug, Args)]
pub struct LessonCaptureArgs {
    /// What kind of failure this is.
    #[arg(long, value_parser = ["stop-rule", "blocked", "flake", "review-ask", "budget", "gate-budget", "main-red"])]
    pub kind: String,
    /// The task where the failure was seen.
    #[arg(long)]
    pub issue: Option<u64>,
    /// Issue title (default: the first line of the failure text).
    #[arg(long)]
    pub title: Option<String>,
    /// The failure text: `-` for stdin, or a file.
    #[arg(long, default_value = "-")]
    pub from: String,
}

#[derive(Debug, Args)]
pub struct LessonTriageArgs {
    /// The lesson issue.
    pub issue: u64,
    /// blocking sets process-red, mechanizable opens a process-fix task, advice goes to the store.
    #[arg(long = "as", value_parser = ["blocking", "mechanizable", "advice"])]
    pub triage: String,
    /// Paths the process-fix task touches (mechanizable; at least one).
    #[arg(long = "touch", value_name = "GLOB")]
    pub touch: Vec<String>,
    /// Area label of the process-fix task.
    #[arg(long)]
    pub area: Option<String>,
    /// The rule text (advice).
    #[arg(long)]
    pub rule: Option<String>,
    /// Scope globs of the advice entry; none makes it global.
    #[arg(long = "scope", value_name = "GLOB")]
    pub scope: Vec<String>,
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
    /// Compare today's spend with the daily budget; at 100 % return the sessions to stop.
    Check,
}

#[derive(Debug, Args)]
pub struct ProgressArgs {
    /// Length of the window in days, ending now.
    #[arg(long, default_value_t = 7)]
    pub days: u32,
    /// Show the lessons view instead: open, blocking, mechanized, hits per week (spec §6).
    #[arg(long, conflicts_with = "days")]
    pub lessons: bool,
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
    /// Import, then close what the plan dropped and update changed tasks (after an amendment merged).
    Amend(PlanAmendArgs),
}

#[derive(Debug, Args)]
pub struct PlanAmendArgs {
    /// The plan file, for example plans/bosun.md.
    pub file: String,
    /// Show what would be created, closed and updated; write nothing.
    #[arg(long)]
    pub dry_run: bool,
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
pub struct IntakeArgs {
    /// What is wanted.
    pub text: String,
    /// Where the request came from.
    #[arg(long, value_parser = ["slack", "lesson", "flake", "human"])]
    pub source: String,
    /// The plan item the request belongs to (sets the area label).
    #[arg(long, value_name = "ID")]
    pub item: Option<String>,
    /// Issue title after `intake: ` (default: the first line of the text).
    #[arg(long)]
    pub title: Option<String>,
    /// The plan file to amend (default plans/<project>.md).
    #[arg(long, value_name = "FILE")]
    pub plan: Option<String>,
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

#[derive(Debug, Args)]
pub struct MainTriageArgs {
    /// A rev where the check passes (older than --bad).
    #[arg(long)]
    pub good: String,
    /// A rev where the check fails (default HEAD).
    #[arg(long)]
    pub bad: Option<String>,
    /// Shell command to run in each worktree; non-zero is bad (default: bosun gate test --all).
    #[arg(long)]
    pub run: Option<String>,
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// A stack to set up (repeatable for a mixed repo).
    #[arg(long = "stack", required = true, value_parser = ["rust", "ts-pnpm"])]
    pub stack: Vec<String>,
    /// A plan file to copy to plans/.
    #[arg(long, value_name = "FILE")]
    pub plan: Option<PathBuf>,
    /// The owner's GitHub login, seeded into governance/owners.toml.
    #[arg(long, value_name = "LOGIN")]
    pub owner: Option<String>,
    /// The splice source of the channel.
    #[arg(long, default_value = "gh:keelstackhq/bosun//channel")]
    pub source: String,
    /// The channel ref (default: v<this version>).
    #[arg(long = "ref", value_name = "REF")]
    pub git_ref: Option<String>,
    /// Pin .bosun/version to this exact X.Y.Z.
    #[arg(long, value_name = "X.Y.Z")]
    pub bosun_version: Option<String>,
    /// Commit on bosun/init but do not push or open a PR.
    #[arg(long)]
    pub no_pr: bool,
}

#[derive(Debug, Args)]
pub struct RevertArgs {
    /// The merged PR to revert.
    pub pr: u64,
}

#[derive(Debug, Args)]
pub struct MainHealthArgs {
    /// The CI conclusion: success, failure, cancelled, skipped, ...
    #[arg(long)]
    pub result: String,
    /// The URL of the run, for the issue and the comment.
    #[arg(long)]
    pub run_url: Option<String>,
}
