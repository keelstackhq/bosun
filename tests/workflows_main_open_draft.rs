//! main-open judges a draft again when the mainfix label changes, so an old verdict never
//! outlives the label until the PR is ready; every other event still skips a draft.

fn judge_if(path: &str) -> String {
    let text = std::fs::read_to_string(path).unwrap();
    let start = text.find("  judge:\n").expect("judge job");
    let rest = &text[start..];
    let end = rest.find("    runs-on:").expect("runs-on");
    rest[..end].split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn a_mainfix_label_judges_a_draft() {
    for path in [
        ".github/workflows/main-open.yml",
        "channel/pieces/core/files/.github/workflows/main-open.yml.jinja",
    ] {
        let cond = judge_if(path);
        assert!(
            cond.contains(
                "(github.event.pull_request.draft == false && github.event.action != 'labeled' && github.event.action != 'unlabeled') || github.event.label.name == 'bosun:kind/mainfix'"
            ),
            "{path}: {cond}"
        );
        let text = std::fs::read_to_string(path).unwrap();
        assert_eq!(
            bosun::core::ci_minutes::check(path, &text),
            vec![],
            "{path}"
        );
    }
}
