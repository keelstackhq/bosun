//! Bisect merged PRs: find the first bad item in a history ordered oldest to newest.

/// The result of a search: the first bad index and every probe, in order.
#[derive(Debug, PartialEq, Eq)]
pub struct Bisect {
    pub first_bad: Option<usize>,
    pub probes: Vec<usize>,
}

/// Binary search. `is_bad` must be monotone: good ... good, bad ... bad. The newest item is probed
/// first; if it is good, there is no bad item (`first_bad` is `None`). Stops at the first `Err`.
pub fn first_bad<T, E, F: FnMut(&T) -> Result<bool, E>>(
    items: &[T],
    mut is_bad: F,
) -> Result<Bisect, E> {
    let mut probes = Vec::new();
    let Some(last) = items.len().checked_sub(1) else {
        return Ok(Bisect {
            first_bad: None,
            probes,
        });
    };
    probes.push(last);
    if !is_bad(&items[last])? {
        return Ok(Bisect {
            first_bad: None,
            probes,
        });
    }
    let (mut lo, mut hi) = (0, last);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        probes.push(mid);
        if is_bad(&items[mid])? {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Ok(Bisect {
        first_bad: Some(lo),
        probes,
    })
}

/// The PR number of a squash subject (`feat: x (#123)`) or a merge subject
/// (`Merge pull request #123 from ...`).
pub fn pr_number(subject: &str) -> Option<u64> {
    let s = subject.trim();
    if let Some(rest) = s.strip_prefix("Merge pull request #") {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        return digits.parse().ok();
    }
    let inner = s.strip_suffix(')')?;
    let start = inner.rfind("(#")?;
    inner[start + 2..].parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(n: usize, first: Option<usize>) -> Bisect {
        let items: Vec<usize> = (0..n).collect();
        first_bad(&items, |i| Ok::<_, ()>(first.is_some_and(|f| *i >= f))).unwrap()
    }

    #[test]
    fn finds_each_position_of_eight() {
        for pos in 0..8 {
            let r = search(8, Some(pos));
            assert_eq!(r.first_bad, Some(pos));
            assert!(r.probes.len() <= 4, "{:?}", r.probes);
        }
    }

    #[test]
    fn all_good_probes_once() {
        let r = search(8, None);
        assert_eq!(r.first_bad, None);
        assert_eq!(r.probes, vec![7]);
    }

    #[test]
    fn empty_and_error() {
        assert_eq!(search(0, None).probes, Vec::<usize>::new());
        let r = first_bad(&[1, 2], |_| Err::<bool, _>("boom"));
        assert_eq!(r.unwrap_err(), "boom");
    }

    #[test]
    fn pr_numbers() {
        assert_eq!(pr_number("feat: x (#123)"), Some(123));
        assert_eq!(pr_number("fix: a (#1) b (#45)"), Some(45));
        assert_eq!(pr_number("Merge pull request #7 from a/b"), Some(7));
        assert_eq!(pr_number("feat: x (#12) more"), None);
        assert_eq!(pr_number("feat: x (#ab)"), None);
        assert_eq!(pr_number("plain"), None);
    }
}
