//! Pure process logic: no file system, no processes, no network, no GitHub client.
//! `tests/core_is_pure.rs` enforces it.

pub mod glob;
pub mod guard;
pub mod lease;
pub mod lessons;
pub mod meta;
pub mod records;
pub mod review;
pub mod risk;
pub mod spawn;
pub mod state;
pub mod stop;
pub mod verdict;
pub mod wave;
