//! What a reviewer needs besides the diff (spec §7): the use cases of the task's epic. The
//! lessons come from `core::lessons`.

/// The `- UC-…` lines of an epic body.
pub fn use_cases(body: &str) -> Vec<String> {
    body.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("- UC-"))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn use_case_lines() {
        let body = "Epic\n- UC-M1-1: a\n  - UC-M1-2: b\n* not\n";
        assert_eq!(use_cases(body), ["- UC-M1-1: a", "- UC-M1-2: b"]);
    }
}
