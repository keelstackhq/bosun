//! `bosun ask <issue> <question>`: mention the owners in a comment and set `needs-human`.
//! There is no timeout and no automatic answer (spec section 8).

use std::path::Path;

use serde_json::json;

use super::AskArgs;
use crate::app::{App, default_branch, kill_switch, set_state};
use crate::cli::risk_check::{OWNERS_PATH, owner_logins};
use crate::core::records::Record;
use crate::core::state::State;
use crate::output::{BosunError, CmdResult, Outcome};

pub fn run(dir: Option<&Path>, args: &AskArgs) -> CmdResult {
    let app = App::load(dir)?;
    let (gh, n) = (app.gh.as_ref(), args.issue);
    kill_switch(gh)?;
    let issue = gh.get_issue(n)?.ok_or_else(|| {
        BosunError::error(
            format!("issue #{n} does not exist"),
            "check the number; `bosun status` lists the tasks",
        )
    })?;
    // The owners are read from the default branch.
    let branch = default_branch(gh)?;
    let owners = owner_logins(gh, &branch)?;
    if owners.is_empty() {
        return Err(BosunError::error(
            format!("no owners to ask: {OWNERS_PATH} is missing or empty on {branch}"),
            format!(
                "fill {OWNERS_PATH} with `[[owner]]` tables (`github = \"login\"`) and merge it"
            ),
        ));
    }
    let mentions: Vec<String> = owners.iter().map(|o| format!("@{o}")).collect();
    let record = Record::Ask {
        session: app.ctx.session.clone(),
        question: args.question.clone(),
    };
    let body = format!(
        "{} {}\n\n{}",
        mentions.join(" "),
        args.question,
        record.to_line()
    );
    let comment = gh.add_comment(n, &body)?;
    set_state(gh, n, &issue, State::NeedsHuman)?;
    Ok(Outcome::data(
        json!({"issue": n, "comment_id": comment.id, "owners": owners}),
    ))
}
