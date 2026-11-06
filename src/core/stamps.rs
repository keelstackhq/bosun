//! Local gate stamps (spec section 4): a passing `bosun gate` leaves a file per tree and gate, and
//! `bosun pr --ready` asks for every ready gate. Pure naming and set logic; the files are read
//! and written by `cli::stamps`.

/// The profile gates a PR must have passed locally before it leaves draft, in run order.
/// `test-integrity` is built in and always required, so it comes first.
pub const READY_GATES: [&str; 6] = ["test-integrity", "fmt", "lint", "types", "test", "deps"];

/// The ready gates for stacks whose profiles define the gates in `per_stack`: `test-integrity`
/// and each other ready gate that some profile has, in `READY_GATES` order.
pub fn required_gates(per_stack: &[Vec<String>]) -> Vec<String> {
    let has = |g: &str| per_stack.iter().any(|gates| gates.iter().any(|x| x == g));
    READY_GATES
        .iter()
        .copied()
        .filter(|g| *g == "test-integrity" || has(g))
        .map(String::from)
        .collect()
}

/// The stamp path below the git common directory: `bosun/gates/<tree>/<gate>`.
pub fn rel_path(tree: &str, gate: &str) -> String {
    format!("bosun/gates/{tree}/{gate}")
}

/// A gate name that is safe as a file name (stamps are written from the CLI argument).
pub fn safe_name(gate: &str) -> bool {
    !gate.is_empty()
        && gate
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The shell loop that stamps `gates`, for the `fix` of a missing-stamps error.
pub fn gate_loop(gates: &[String]) -> String {
    format!(
        "for g in {}; do bosun gate $g || break; done",
        gates.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn required_gates_are_the_profile_gates_in_run_order() {
        let rust = names(&["fmt", "lint", "test", "deps", "mutants"]);
        assert_eq!(
            required_gates(std::slice::from_ref(&rust)),
            names(&["test-integrity", "fmt", "lint", "test", "deps"])
        );
        let ts = names(&["types", "fmt", "snapshots"]);
        assert_eq!(
            required_gates(&[rust, ts]),
            names(&["test-integrity", "fmt", "lint", "types", "test", "deps"])
        );
        assert_eq!(required_gates(&[]), names(&["test-integrity"]));
    }

    #[test]
    fn names_and_paths() {
        assert_eq!(rel_path("abc", "fmt"), "bosun/gates/abc/fmt");
        assert!(safe_name("test-integrity") && !safe_name("../x") && !safe_name(""));
        assert_eq!(
            gate_loop(&names(&["fmt", "lint"])),
            "for g in fmt lint; do bosun gate $g || break; done"
        );
    }
}
