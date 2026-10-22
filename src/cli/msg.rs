//! `bosun msg`: return a `send_message` call that steers another session (spec §5). Nothing
//! here calls GitHub, and nothing waits for a reply.

use std::path::Path;

use serde_json::json;

use super::MsgArgs;
use crate::ctx::Ctx;
use crate::output::{BosunError, CmdResult, Outcome};

/// Longest message in characters.
pub const MAX_CHARS: usize = 500;

pub fn run(dir: Option<&Path>, args: &MsgArgs) -> CmdResult {
    let len = args.text.chars().count();
    if args.text.trim().is_empty() {
        return Err(BosunError::usage(
            "the message is empty",
            "pass the text to send as the second argument",
        ));
    }
    if len > MAX_CHARS {
        return Err(BosunError::usage(
            format!("the message has {len} characters, the limit is {MAX_CHARS}"),
            "shorten it, and put the details in a comment on the issue",
        ));
    }
    let from = match args.issue {
        Some(n) => format!("#{n}"),
        None => Ctx::load(dir)?.session,
    };
    Ok(
        Outcome::data(json!({"session": args.session, "chars": len})).with_next(
            "send_message",
            json!({"session_id": args.session, "message": format!("From {from}: {}", args.text)}),
        ),
    )
}
