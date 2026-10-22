#![forbid(unsafe_code)]
//! bosun: orchestration helper for repos where agents write all the code. See `docs/spec.md`.

pub mod output;

pub use output::{BosunError, CmdResult, Exit, Next, Outcome, envelope};
