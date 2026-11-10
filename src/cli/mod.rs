//! Command-line interface: one subcommand per bosun command (spec §11 v0.1 list, plus msg and
//! budget record). Each command module returns a `CmdResult`.

use clap::{Args, Parser, Subcommand};

pub mod adopt;
pub mod answer;
pub mod approve;
pub mod approve_chat;
pub mod ask;
pub mod budget;
mod budget_pr;
pub mod budget_report;
pub mod carry_over;
pub mod ci_log;
pub mod claim;
pub mod doctor;
pub mod handoff;
pub mod handoff_pr;
pub mod heartbeat;
pub mod hold;
pub mod init;
pub mod intake;
pub mod intake_file;
pub mod join;
pub mod keygen;
pub mod lesson;
pub mod lesson_triage;
pub mod main_triage;
pub mod merge;
pub mod msg;
pub mod notify;
pub mod pin;
pub mod pin_pr;
pub mod plan;
pub mod plan_pr;
pub mod pr;
mod pr_bundle;
pub mod pr_no_ci;
pub mod prereqs;
pub mod prereqs_gate;
pub mod progress;
pub mod reap;
pub mod release;
pub mod result;
pub mod resume;
pub mod revert;
pub mod review_pack;
pub mod risk_check;
pub mod spawn;
pub mod spawn_relay;
pub mod stamps;
pub mod status;
pub mod trailers;
pub mod verdict;
pub mod wait;
pub mod wave;
use std::path::PathBuf;

pub mod ci_minutes;
pub mod gate;
pub mod gate_missed;
pub mod main_health;
pub mod review_ci;
pub mod setup_snippet;
pub mod stop;
pub mod test_integrity;
pub mod uninstall;

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
    /// Owner only (BOSUN_OWNER_TOKEN): list the PRs that wait for an owner and approve them.
    Approve(ApproveArgs),
    /// Make an ed25519 approval key (PEM, mode 0600, outside the repo); `--enroll` adds it to .bosun/approvers.toml.
    Keygen(KeygenArgs),
    /// Orchestrator only: re-check every gate on the PR's head and merge it (spec D13, D-019).
    Merge(MergeArgs),
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
    /// Record a human step found mid-run: the owner's checklist gets a box, the task waits.
    Hold(HoldArgs),
    /// Record an owner's answer from Slack on a needs-human task and make it ready.
    Answer(AnswerArgs),
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
    Resume(ResumeArgs),
    /// Write this root's handoff (at most 40 lines) to the registry for the next root.
    Handoff(HandoffArgs),
    /// Return a send_message call telling your parent that PR N is ready (head SHA, risk paths, review minors).
    #[command(name = "handoff-pr")]
    HandoffPr(HandoffPrArgs),
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
    /// Commit trailers: stamp (commit-msg hook), install the hook, list a session's commits.
    Trailers(TrailersArgs),
    /// Open a kind/plan issue that asks for a plan amendment (the only way new work enters).
    Intake(IntakeArgs),
    /// Open or close the main-red issue from a CI result; re-judge the open PRs (spec §9).
    MainHealth(MainHealthArgs),
    /// Open (or reuse) the "Which gate missed this?" task for an escaped defect (spec §9).
    GateMissed(GateMissedArgs),
    /// Review mode b: ask the Anthropic API for the verdict of a PR and record it (no session).
    ReviewCi(ReviewCiArgs),
    /// List the Slack posts that are due (channel check first); `--ack KEY` records a post.
    Notify(NotifyArgs),
    /// List the human steps a plan needs up front (and its approvals); --post opens the checklist issue.
    Prereqs(PrereqsArgs),
    /// Write bosun's files into this worktree without splice, hull or hoist; no commit, no PR.
    Adopt(AdoptArgs),
    /// Take back what `bosun adopt` wrote (only files whose bytes are still adopt's).
    Uninstall(UninstallArgs),
    /// Move the tool pins (setup script, .bosun/version, channel template) to the newest verified npm release.
    Pin(PinArgs),
    /// Print the bootstrap to paste once as a cloud environment's setup script.
    SetupSnippet,
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
    "approve",
    "risk-check",
    "test-integrity",
    "review-pack",
    "verdict",
    "pr",
    "ci-log",
    "status",
    "ask",
    "hold",
    "lesson",
    "hook",
    "doctor",
    "budget",
    "plan",
    "fail",
    "resume",
    "handoff",
    "handoff-pr",
    "progress",
    "result",
    "main-triage",
    "init",
    "revert",
    "trailers",
    "intake",
    "main-health",
    "gate-missed",
    "review-ci",
    "notify",
    "answer",
    "prereqs",
    "adopt",
    "uninstall",
    "pin",
    "setup-snippet",
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
            Command::Approve(_) => "approve",
            Command::Keygen(_) => "keygen",
            Command::Merge(_) => "merge",
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
            Command::Hold(_) => "hold",
            Command::Answer(_) => "answer",
            Command::Lesson(LessonCmd::Add(_)) => "lesson add",
            Command::Lesson(LessonCmd::Hit(_)) => "lesson hit",
            Command::Lesson(LessonCmd::Capture(_)) => "lesson capture",
            Command::Lesson(LessonCmd::Triage(_)) => "lesson triage",
            Command::Hook(HookCmd::PreToolUse) => "hook pre-tool-use",
            Command::Hook(HookCmd::PostToolUse) => "hook post-tool-use",
            Command::Hook(HookCmd::SessionStart) => "hook session-start",
            Command::Hook(HookCmd::Stop) => "hook stop",
            Command::Doctor(_) => "doctor",
            Command::Budget(BudgetCmd::Record(_)) => "budget record",
            Command::Plan(PlanCmd::Check(_)) => "plan check",
            Command::Plan(PlanCmd::Import(_)) => "plan import",
            Command::Plan(PlanCmd::Amend(_)) => "plan amend",
            Command::Plan(PlanCmd::Pr(_)) => "plan pr",
            Command::Fail(_) => "fail",
            Command::Resume(_) => "resume",
            Command::Handoff(_) => "handoff",
            Command::HandoffPr(_) => "handoff-pr",
            Command::Progress(_) => "progress",
            Command::Budget(BudgetCmd::Check) => "budget check",
            Command::Budget(BudgetCmd::Report(_)) => "budget report",
            Command::Result(_) => "result",
            Command::MainTriage(_) => "main-triage",
            Command::Init(_) => "init",
            Command::Adopt(_) => "adopt",
            Command::Uninstall(_) => "uninstall",
            Command::Pin(_) => "pin",
            Command::SetupSnippet => "setup-snippet",
            Command::Revert(_) => "revert",
            Command::Trailers(_) => "trailers",
            Command::Intake(a) => match a.cmd {
                None => "intake",
                Some(IntakeCmd::Check(_)) => "intake check",
                Some(IntakeCmd::Sign(_)) => "intake sign",
                Some(IntakeCmd::Verify(_)) => "intake verify",
            },
            Command::MainHealth(_) => "main-health",
            Command::GateMissed(_) => "gate-missed",
            Command::ReviewCi(_) => "review-ci",
            Command::Notify(_) => "notify",
            Command::Prereqs(_) => "prereqs",
        }
    }
}

#[derive(Debug, Args)]
pub struct ResumeArgs {
    /// Also return the newest handoff of this root lineage in `data.handoff`.
    #[arg(long)]
    pub handoff: bool,
}

#[derive(Debug, Args)]
pub struct HandoffArgs {
    /// File with the `<kind>: <text>` lines (wave, pr, lease, next, blocker), or `-` for stdin.
    #[arg(long, value_name = "FILE")]
    pub from: Option<String>,
}

#[derive(Debug, Args)]
pub struct HandoffPrArgs {
    /// The PR number.
    pub pr: u64,
    /// The parent session; default: the parent of your `bosun join` record, else the orchestrator of a `<id>/T*` id.
    #[arg(long, value_name = "SESSION")]
    pub to: Option<String>,
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
    /// Claude Tag seat: the Slack channel id (needs --origin slack; must be `[slack] channel`).
    #[arg(long, value_name = "C...")]
    pub slack_channel: Option<String>,
    /// Claude Tag seat: the thread ts (needs --slack-channel).
    #[arg(long, value_name = "TS")]
    pub slack_thread: Option<String>,
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
    /// How the work ended (`partial`: some tasks merged, the rest goes back to the parent).
    #[arg(long, value_parser = RESULT_OUTCOMES)]
    pub outcome: String,
    /// The PR of the work.
    #[arg(long)]
    pub pr: Option<u64>,
}

#[derive(Debug, Args)]
pub struct SpawnArgs {
    /// The epic issue number (optional only with --slack-thread).
    pub epic: Option<u64>,
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
    /// The Slack thread ts that asked for this work (needs --slack-user); stored in the lease.
    #[arg(long, value_name = "TS")]
    pub slack_thread: Option<String>,
    /// The Slack user ID of the requester, from the Claude Tag event (an owner's `slack` ID).
    #[arg(long, value_name = "U...")]
    pub slack_user: Option<String>,
    /// With --slack-thread and nothing ready: the request for intake, `-` for stdin, or a file.
    #[arg(long, value_name = "FILE")]
    pub request: Option<String>,
    /// Relay: a fresh root session that continues this root's lineage (with --handoff, or --record).
    #[arg(long)]
    pub root: bool,
    /// With --root: return the create_session call for the next root (after `bosun handoff`).
    #[arg(long)]
    pub handoff: bool,
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
    #[arg(long, default_value_t = WAIT_TIMEOUT_SECS)]
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
    /// fmt, lint, types, test, deps, mutants, snapshots, test-integrity, ci-minutes, main-open, ...
    /// `ci-minutes` is built in: it checks `.github/workflows` for the CI-minute rules (drafts skip,
    /// `ready_for_review`, cancelling concurrency, `paths` on heavy PR jobs).
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
pub struct MergeArgs {
    /// The PR number.
    pub pr: u64,
    /// GitHub merge method.
    #[arg(long, value_parser = ["squash", "merge", "rebase"], default_value = "squash")]
    pub method: String,
}

#[derive(Debug, Args)]
pub struct KeygenArgs {
    /// Where to write the private key PEM (default ~/.bosun/approver.pem). Never overwritten, and
    /// never inside any git worktree. A relative path is relative to `-C` when given.
    #[arg(long, value_name = "PEM")]
    pub out: Option<std::path::PathBuf>,
    /// Key id for .bosun/approvers.toml (default `<role>-<YYYY-MM>`); A-Z a-z 0-9 . _ - only.
    #[arg(long)]
    pub id: Option<String>,
    /// The key's role: `owner` (default) or `delegate`.
    #[arg(long, value_parser = ["owner", "delegate"])]
    pub role: Option<String>,
    /// Append the key to <repo>/.bosun/approvers.toml (created when missing).
    #[arg(long)]
    pub enroll: bool,
}

#[derive(Debug, Args)]
pub struct ApproveArgs {
    /// `<pr>@<40-hex head sha>`, copied from `data.approve_command` of `--list`; nothing is
    /// posted (exit 3) unless every PR is ready and still at that head.
    #[arg(value_name = "PR@HEAD", conflicts_with_all = ["list", "main_open"])]
    pub prs: Vec<String>,
    /// List the open PRs that wait for an owner and are ready; writes nothing.
    #[arg(long, conflicts_with = "main_open")]
    pub list: bool,
    /// Post the `main-open` status on this PR (bootstrap: the PR changes main-open.yml).
    #[arg(long, value_name = "PR")]
    pub main_open: Option<u64>,
    /// Sign the approval of one `<pr>@<head>` (the head the owner approved; it must still be
    /// the PR's head) with this ed25519 PKCS#8 PEM key, enrolled in .bosun/approvers.toml.
    #[arg(long, value_name = "PEM", conflicts_with_all = ["list", "main_open"])]
    pub sign_key: Option<std::path::PathBuf>,
    /// With --sign-key: print the comment body in data.body instead of posting it.
    #[arg(long, requires = "sign_key")]
    pub print: bool,
    /// With --sign-key: sign as `owner` (default) or `delegate`; it must be the key's enrolled
    /// role.
    #[arg(long = "as", value_name = "ROLE", requires = "sign_key", value_parser = ["owner", "delegate"])]
    pub role: Option<String>,
    /// With --sign-key: sign in bootstrap mode (D-012), until `[approvals] bootstrap_until` on
    /// the default branch; an owner key only.
    #[arg(long, requires = "sign_key")]
    pub bootstrap: bool,
    /// Record the owner's chat answer for one `<pr>@<head>` as a chat approval comment (an
    /// orchestrator only; spec section 8).
    #[arg(long, conflicts_with_all = ["list", "main_open", "sign_key"])]
    pub chat: bool,
    /// With --chat: the question id the owner answered (`data.ready[].ask.question_id`).
    #[arg(
        long,
        value_name = "ID",
        requires = "chat",
        conflicts_with = "slack_user"
    )]
    pub question: Option<String>,
    /// With --chat: the owner's Slack user ID who answered in the Claude Tag thread.
    #[arg(long, value_name = "U...", requires = "chat")]
    pub slack_user: Option<String>,
}

#[derive(Debug, Args)]
pub struct ReviewCiArgs {
    /// The PR number.
    pub pr: u64,
    /// Run up to the parsed verdict and write nothing to GitHub; print the verdict in data.
    #[arg(long)]
    pub dry_run: bool,
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
    /// Mark the draft PR ready for review (it needs the local HEAD pushed and every ready gate
    /// stamped by `bosun gate`) and turn on auto-merge (bosun never merges). `--auto-merge` is
    /// the same flag.
    #[arg(long, alias = "auto-merge")]
    pub ready: bool,
    /// Another issue of the lead's single-shape bundle (repeatable): the PR closes it too.
    #[arg(long = "bundle", value_name = "ISSUE")]
    pub bundle: Vec<u64>,
}

#[derive(Debug, Args)]
pub struct CiLogArgs {
    pub pr: u64,
    /// Lines kept from the start of each log.
    #[arg(long, default_value_t = 20)]
    pub head: usize,
    /// Lines kept from the end of each log.
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

#[derive(Debug, Args)]
pub struct HoldArgs {
    /// The task issue number.
    pub issue: u64,
    /// What the owner must do (at most 300 characters).
    pub text: String,
    /// The owner can do it on a phone.
    #[arg(long)]
    pub phone: bool,
}

#[derive(Debug, Args)]
pub struct AnswerArgs {
    /// The task issue number.
    pub issue: u64,
    /// The Slack user ID of the answerer, from the Claude Tag event (an owner's `slack` ID).
    #[arg(long, required_unless_present = "chat", conflicts_with = "chat")]
    pub slack_user: Option<String>,
    /// The answer is the chat form the orchestrator relays, signed by a key enrolled in
    /// .bosun/approvers.toml; it is verified before it is recorded.
    #[arg(long)]
    pub chat: bool,
    /// With --chat: sign the owner's plain answer with this ed25519 PKCS#8 PEM key and print
    /// the chat form in data.chat; nothing is posted.
    #[arg(long, value_name = "PEM", conflicts_with = "slack_user")]
    pub sign_key: Option<PathBuf>,
    /// The answer text: `-` for stdin, or a file.
    #[arg(long, default_value = "-")]
    pub from: String,
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
    /// The rule, at most 160 characters; the same rule as an entry records a hit on it.
    pub text: String,
    #[arg(long = "scope", value_name = "GLOB")]
    pub scope: Vec<String>,
    #[arg(long)]
    pub why: Option<String>,
    /// The PR or issue that proved it (`#N`); required unless --upstream.
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
    /// PostToolUse: store the owner's answer to a bosun AskUserQuestion.
    PostToolUse,
    /// SessionStart: join and print the session context.
    SessionStart,
    /// Stop: block once while the lease has no result or handoff; warn on a stale gate.
    Stop,
}

#[derive(Debug, Subcommand)]
pub enum BudgetCmd {
    /// Record the spend of a task (claude-sessions or generic provider).
    Record(BudgetRecordArgs),
    /// Compare today's spend with the daily budget; at 100 % return the sessions to stop.
    Check,
    /// One month of spend per PR, with totals; `--out DIR` also writes DIR/YYYY-MM.md and .json.
    Report(BudgetReportArgs),
}

#[derive(Debug, Args)]
pub struct BudgetReportArgs {
    /// The month as YYYY-MM (UTC); default: the current month.
    #[arg(long)]
    pub month: Option<String>,
    /// Write DIR/YYYY-MM.md and DIR/YYYY-MM.json.
    #[arg(long)]
    pub out: Option<std::path::PathBuf>,
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
    /// The task issue (leave out with --pr).
    #[arg(conflicts_with = "pr")]
    pub issue: Option<u64>,
    /// A PR the session made (repeatable): the cost is split evenly over the PRs and recorded on each.
    #[arg(long, value_name = "N", conflicts_with = "issue")]
    pub pr: Vec<u64>,
    /// With --pr: the cost in USD, instead of reading it from the session.
    #[arg(long, requires = "pr")]
    pub usd: Option<f64>,
    /// With --pr: the session that spent it (an orchestrator recording for a child); default: the id in the get_session result, else the caller.
    #[arg(long, requires = "pr")]
    pub session: Option<String>,
    #[arg(long, default_value = "claude-sessions", value_parser = ["claude-sessions", "generic"])]
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
    /// Open the PR of a plan amendment in one step: intake, claim of the plan file, `bosun pr`.
    Pr(PlanPrArgs),
}

#[derive(Debug, Args)]
pub struct PlanPrArgs {
    /// The plan file, repo-relative, for example plans/bosun.md.
    pub file: String,
    /// Why the plan changes (the intake text).
    pub why: String,
    /// Where the request came from.
    #[arg(long, value_parser = ["slack", "lesson", "flake", "human"])]
    pub source: String,
    /// The plan item the amendment belongs to (sets the area label).
    #[arg(long, value_name = "ID")]
    pub item: Option<String>,
    /// Intake title after `intake: ` (default: the first line of the text).
    #[arg(long)]
    pub title: Option<String>,
    /// Also turn on auto-merge (as `bosun pr --auto-merge`; bosun never merges).
    #[arg(long)]
    pub auto_merge: bool,
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
pub struct PrereqsArgs {
    /// The plan files, for example plans/bosun.md.
    #[arg(required = true)]
    pub files: Vec<String>,
    /// Create or update the checklist issue of each plan.
    #[arg(long)]
    pub post: bool,
    /// Leave out the `bosun doctor` steps.
    #[arg(long)]
    pub no_setup: bool,
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
    /// Keep plan tasks open even when the plan file on the default branch is byte-identical to
    /// this one (by default they are closed: there is nothing left to commit).
    #[arg(long)]
    pub keep_plan_tasks_open: bool,
}

/// `intake "<text>" --source ...` opens a plan issue; a text of exactly `check` is the subcommand.
#[derive(Debug, Args)]
#[command(args_conflicts_with_subcommands = true)]
pub struct IntakeArgs {
    #[command(subcommand)]
    pub cmd: Option<IntakeCmd>,
    /// What is wanted (required without a subcommand).
    #[arg(default_value = "")]
    pub text: String,
    /// Where the request came from (required without a subcommand).
    #[arg(long, default_value = "", value_parser = parse_source)]
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

/// Refuse text that looks like a secret (exit 2). The error names the kind and `field`, never
/// the matched text (spec §6).
pub(crate) fn refuse_secret(field: &str, text: &str) -> Result<(), crate::output::BosunError> {
    match crate::core::text::secret_kind(text) {
        None => Ok(()),
        Some(kind) => Err(crate::output::BosunError::usage(
            format!("{field} looks like a secret ({kind}); bosun does not store or post it"),
            "redact the secret (replace it with <redacted>) and run again; never put tokens, keys or PEM in bosun text",
        )),
    }
}

/// [`refuse_secret`] for every value of a repeatable option.
pub(crate) fn refuse_secrets(
    field: &str,
    items: &[String],
) -> Result<(), crate::output::BosunError> {
    items.iter().try_for_each(|t| refuse_secret(field, t))
}

/// `text` and `--source` default to empty; `intake::args_ok` refuses an empty one.
fn parse_source(s: &str) -> Result<String, String> {
    match s {
        "" | "slack" | "lesson" | "flake" | "human" => Ok(s.to_string()),
        _ => Err("expected one of slack, lesson, flake, human".into()),
    }
}

#[derive(Debug, Subcommand)]
pub enum IntakeCmd {
    /// Validate the intake file against the schema (no signature handling).
    Check(IntakeCheckArgs),
    /// Sign an intake file with an ed25519 key; prints the detached .bosun/intake.sig, writes nothing.
    Sign(IntakeSignArgs),
    /// For provision.yml only: verify a file against its detached signature and the approvers file.
    Verify(IntakeVerifyArgs),
}

#[derive(Debug, Args)]
pub struct IntakeCheckArgs {
    /// The intake file (default .bosun/intake.toml).
    pub file: Option<PathBuf>,
    /// Read the file from stdin (`-`) instead.
    #[arg(long, value_name = "-", value_parser = ["-"], conflicts_with = "file")]
    pub from: Option<String>,
}

#[derive(Debug, Args)]
pub struct IntakeSignArgs {
    /// The intake file to sign.
    pub file: PathBuf,
    /// ed25519 PKCS#8 PEM private key (read from this path only).
    #[arg(long, value_name = "PEM")]
    pub sign_key: PathBuf,
    /// The repo being provisioned, owner/name; it must equal repo.owner/repo.name of the file.
    #[arg(long, value_name = "OWNER/NAME")]
    pub repo: String,
    /// new (create the repo) or adopt (prepare an existing one); part of the signed message.
    #[arg(long, value_parser = ["new", "adopt"])]
    pub mode: String,
    /// The key id (default: the id of the key in the approvers file that has this public key).
    #[arg(long)]
    pub key_id: Option<String>,
    /// Seconds until the signature expires (1 to 3600).
    #[arg(long, default_value_t = 3600)]
    pub expires_in: u64,
    /// Approvers file for the key id lookup (default .bosun/approvers.toml).
    #[arg(long, value_name = "PATH")]
    pub approvers: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct IntakeVerifyArgs {
    /// The intake file.
    pub file: PathBuf,
    /// The detached signature file.
    #[arg(long, value_name = "SIGFILE")]
    pub sig: PathBuf,
    /// The approvers file to verify against (provision's own checked-out main).
    #[arg(long, value_name = "PATH")]
    pub approvers: PathBuf,
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

#[derive(Debug, Clone, Args)]
pub struct InitArgs {
    /// A stack to set up (repeatable for a mixed repo); with --intake it comes from the file.
    #[arg(long = "stack", required_unless_present = "intake", value_parser = ["rust", "ts-pnpm"])]
    pub stack: Vec<String>,
    /// The owner's intake file: derives --stack, --owner and --plan, becomes .bosun/intake.toml.
    #[arg(long, value_name = "FILE")]
    pub intake: Option<PathBuf>,
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
    /// The setup branch (default: claude/bosun-init in a Claude cloud session, else bosun/init).
    #[arg(long, value_name = "NAME")]
    pub branch: Option<String>,
    /// Commit on the branch but do not push or open a PR; returns pr_title and pr_body.
    #[arg(long)]
    pub no_pr: bool,
}

#[derive(Debug, Args)]
pub struct AdoptArgs {
    /// A stack (repeatable): rust, ts-pnpm or other. Default: detected from the repo root.
    #[arg(long = "stack", value_parser = ["rust", "ts-pnpm", "other"])]
    pub stack: Vec<String>,
    /// The owner's GitHub login, seeded into governance/owners.toml.
    #[arg(long, value_name = "LOGIN")]
    pub owner: Option<String>,
    /// Pin .bosun/version to this X.Y.Z (default: this binary's version).
    #[arg(long, value_name = "X.Y.Z")]
    pub bosun_version: Option<String>,
    /// Write `[approvals] policy` into .bosun/bosun.toml (spec §8); the pinned release must know it.
    #[arg(long, value_parser = ["owner", "owner-gated", "orchestrator", "autonomous"])]
    pub policy: Option<String>,
    /// Write `[ci] provider` (spec §9). Default: `circleci` when the repo has `.circleci/` and no
    /// Actions workflows, else `github`. `none` installs no GitHub workflows. Needs pin 0.12.0+.
    #[arg(long, value_parser = ["github", "circleci", "none"])]
    pub ci: Option<String>,
    /// Show what would change; write nothing and change nothing on GitHub.
    #[arg(long)]
    pub dry_run: bool,
    /// Skip the labels and the registry issue; needs no token and no GitHub repo.
    #[arg(long)]
    pub no_remote: bool,
}

#[derive(Debug, Args)]
pub struct UninstallArgs {
    /// Also delete the labels adopt created (not those still on an open issue) and close the registry issue it opened.
    #[arg(long)]
    pub labels: bool,
    /// List what would change; touch nothing.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Args)]
pub struct TrailersArgs {
    #[command(subcommand)]
    pub cmd: TrailersCmd,
}

#[derive(Debug, Subcommand)]
pub enum TrailersCmd {
    /// Append the missing trailers to a commit message file (the commit-msg hook calls this).
    Stamp {
        /// The message file git passes to the hook.
        file: std::path::PathBuf,
    },
    /// Write `.git/hooks/commit-msg` so every commit here is stamped (idempotent).
    Install,
    /// List the commits of one session, newest first, with a rollback recipe.
    List {
        /// The session id (the `Bosun-Session` trailer value).
        session: String,
    },
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

#[derive(Debug, Args)]
pub struct GateMissedArgs {
    /// The reverted or escaped PR.
    #[arg(long)]
    pub pr: Option<u64>,
    /// The URL of the failing run.
    #[arg(long)]
    pub run: Option<String>,
    /// The commit range of the incident, `GOOD..BAD`.
    #[arg(long)]
    pub range: Option<String>,
}

#[derive(Debug, Args)]
pub struct NotifyArgs {
    /// The channel facts read with the Slack tool, as JSON: `-` for stdin or a file.
    #[arg(long, value_name = "FILE")]
    pub channel_info: Option<String>,
    /// Record that the post with this key was made.
    #[arg(long, value_name = "KEY")]
    pub ack: Option<String>,
    /// List only the thread-results of this Slack thread (`^[0-9]+\.[0-9]+$`).
    #[arg(long, value_name = "TS")]
    pub thread: Option<String>,
    /// Accepted for symmetry; the output is always JSON.
    #[arg(long)]
    pub json: bool,
}

/// The outcomes `bosun result` accepts.
pub const RESULT_OUTCOMES: [&str; 4] = ["merged", "partial", "blocked", "needs-human"];
/// Default `bosun wait --timeout` in seconds.
pub const WAIT_TIMEOUT_SECS: u64 = 1800;

#[derive(Debug, Args)]
pub struct PinArgs {
    /// Move the tool pins to the newest npm release (required; the only mode).
    #[arg(long, required = true)]
    pub latest: bool,
    /// Only this tool (repeatable); default: splice, hoist, berth and bosun.
    #[arg(long, value_name = "TOOL", value_parser = ["splice", "hoist", "berth", "bosun"])]
    pub tool: Vec<String>,
    /// List the tools that are behind; download no tarball, write nothing.
    #[arg(long)]
    pub check: bool,
    /// Also branch `bosun/pin`, commit, push and open the draft PR; prints what is left for you.
    #[arg(long, conflicts_with = "check")]
    pub pr: bool,
}
