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

/// Group files (name, patch bytes) in the given order into chunks of at most `max_bytes` each,
/// for one reviewer subagent per chunk. A file over `max_bytes` gets a chunk of its own.
pub fn chunks(files: &[(String, usize)], max_bytes: usize) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut used = 0usize;
    for (name, size) in files {
        match out.last_mut() {
            Some(c) if used + size <= max_bytes => {
                c.push(name.clone());
                used += size;
            }
            _ => {
                out.push(vec![name.clone()]);
                used = *size;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn use_case_lines() {
        let body = "Epic\n- UC-M1-1: a\n  - UC-M1-2: b\n* not\n";
        assert_eq!(use_cases(body), ["- UC-M1-1: a", "- UC-M1-2: b"]);
    }

    fn f(v: &[(&str, usize)]) -> Vec<(String, usize)> {
        v.iter().map(|(n, s)| (n.to_string(), *s)).collect()
    }

    #[test]
    fn chunks_fill_greedily_in_order() {
        assert!(chunks(&[], 10).is_empty());
        let c = chunks(&f(&[("a", 4), ("b", 5), ("c", 2), ("d", 9)]), 10);
        assert_eq!(c, [vec!["a", "b"], vec!["c"], vec!["d"]]);
        let c = chunks(&f(&[("a", 3), ("b", 3), ("c", 4)]), 10);
        assert_eq!(c, [vec!["a", "b", "c"]]);
    }

    #[test]
    fn a_big_file_is_alone_and_no_chunk_is_empty() {
        let c = chunks(&f(&[("s", 1), ("big", 50), ("t", 1), ("z", 0)]), 10);
        assert_eq!(c, [vec!["s"], vec!["big"], vec!["t", "z"]]);
        assert!(chunks(&f(&[("big", 50)]), 10).iter().all(|c| !c.is_empty()));
    }
}
