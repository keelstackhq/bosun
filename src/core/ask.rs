//! The owner question for `bosun approve --list` (spec section 8, approval batches): one `ask` per
//! ready PR and the matching `AskUserQuestion` call, or a Claude Tag thread post. Pure.

use serde_json::{Value, json};

use crate::core::approve::question_id;
use crate::core::text::inert;

/// AskUserQuestion takes at most 4 options: this many PRs plus `Hold`.
pub const MAX_ASKED: usize = 3;
const TITLE_MAX: usize = 60;
const VERDICT_MAX: usize = 80;

/// One ready PR as the question shows it.
pub struct Item<'a> {
    pub pr: u64,
    pub title: &'a str,
    pub head: &'a str,
    pub risk: &'a [String],
    pub verdict: &'a str,
}

fn short(head: &str) -> String {
    head.chars().take(7).collect()
}

/// The question id, the question text and the two options of one PR.
pub fn ask(repo: &str, item: &Item, unix: i64) -> Value {
    let id = question_id(item.pr, item.head, unix);
    json!({
        "question_id": id,
        "question": format!("Approve {}? [bosun:{id}]", line(repo, item)),
        "options": [format!("Approve {}", short(item.head)), "Hold"],
    })
}

/// `acme/widget#12 `title` (risk R-gov; head abc1234 = <full>; verdict: approved)`.
fn line(repo: &str, item: &Item) -> String {
    let risk = match item.risk {
        [] => "none named".to_string(),
        r => r.join(", "),
    };
    format!(
        "{repo}#{} {} (risk {risk}; head {} = {}; verdict: {})",
        item.pr,
        inert(item.title, TITLE_MAX),
        short(item.head),
        item.head,
        inert(item.verdict, VERDICT_MAX),
    )
}

/// The `AskUserQuestion` arguments for the first `MAX_ASKED` items: one question, multi-select
/// when it names more than one PR. `None` when nothing is ready.
pub fn ask_user_question(repo: &str, items: &[Item], unix: i64) -> Option<Value> {
    let asked = &items[..items.len().min(MAX_ASKED)];
    let (question, multi) = match asked {
        [] => return None,
        [one] => (ask(repo, one, unix)["question"].clone(), false),
        many => {
            let lines: Vec<String> = many
                .iter()
                .map(|i| {
                    let id = question_id(i.pr, i.head, unix);
                    format!("- {} [bosun:{id}]", line(repo, i))
                })
                .collect();
            let text = format!(
                "Approve which PRs? Hold approves none.\n{}",
                lines.join("\n")
            );
            (json!(text), true)
        }
    };
    let mut options: Vec<Value> = asked
        .iter()
        .map(|i| {
            json!({"label": format!("Approve #{} {}", i.pr, short(i.head)),
                "description": format!("Approve {repo}#{} at head {}", i.pr, i.head)})
        })
        .collect();
    options.push(json!({"label": "Hold",
        "description": "Approve nothing now; not asked again for these heads"}));
    Some(
        json!({"questions": [{"question": question, "header": "Approve",
        "multiSelect": multi, "options": options}]}),
    )
}

/// The Claude Tag thread post for the same PRs: it ends with the answer to give.
pub fn slack_text(repo: &str, items: &[Item]) -> Option<String> {
    if items.is_empty() {
        return None;
    }
    let lines: Vec<String> = items
        .iter()
        .map(|i| format!("- {}", line(repo, i)))
        .collect();
    let shorts: Vec<String> = items.iter().map(|i| short(i.head)).collect();
    Some(format!(
        "Waiting for your approval:\n{}\nTo approve, answer \"approve {}\"",
        lines.join("\n"),
        shorts.join(" ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: &str = "abcdef1234567890abcdef1234567890abcdef12";

    fn item<'a>(pr: u64, risk: &'a [String]) -> Item<'a> {
        Item {
            pr,
            title: "t `x`",
            head: H,
            risk,
            verdict: "approved",
        }
    }

    #[test]
    fn header_fits_and_none_when_empty() {
        assert!(ask_user_question("a/b", &[], 1).is_none());
        assert!(slack_text("a/b", &[]).is_none());
        let r = vec!["R-gov".to_string()];
        let v = ask_user_question("a/b", &[item(1, &r)], 1).unwrap();
        assert!(
            v["questions"][0]["header"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                <= 12
        );
    }

    #[test]
    fn caps_at_three_prs_plus_hold() {
        let r: Vec<String> = vec![];
        let items: Vec<_> = (1..=5).map(|n| item(n, &r)).collect();
        let v = ask_user_question("a/b", &items, 1).unwrap();
        assert_eq!(v["questions"][0]["options"].as_array().unwrap().len(), 4);
        assert!(
            v["questions"][0]["question"]
                .as_str()
                .unwrap()
                .contains("risk none named")
        );
    }
}
