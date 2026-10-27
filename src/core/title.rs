//! PR titles: a commit subject is used as the title when it is a conventional commit.

const TYPES: [&str; 10] = [
    "feat", "fix", "docs", "chore", "refactor", "test", "ci", "build", "perf", "style",
];

/// Whether `subject` is `type(scope)!: subject` with a known type (`revert` too).
pub fn is_conventional(subject: &str) -> bool {
    let Some((head, rest)) = subject.split_once(": ") else {
        return false;
    };
    if rest.trim().is_empty() {
        return false;
    }
    let head = head.strip_suffix('!').unwrap_or(head);
    let ty = match head.split_once('(') {
        Some((ty, scope)) => match scope.strip_suffix(')') {
            Some(s) if !s.is_empty() && !s.contains(['(', ')']) => ty,
            _ => return false,
        },
        None => head,
    };
    ty == "revert" || TYPES.contains(&ty)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_conventional_subjects() {
        for s in [
            "feat: add x",
            "fix(pr): trim the name",
            "feat!: drop y",
            "refactor(core)!: split z",
            "revert: undo it",
            "ci(release): pin splice",
        ] {
            assert!(is_conventional(s), "{s}");
        }
    }

    #[test]
    fn rejects_the_rest() {
        for s in [
            "x",
            "Trim the name",
            "intake: C2 hello trims the name",
            "feat:no space",
            "feat: ",
            "fix(): empty scope",
            "fix(a: b",
            "Fix: capitalised",
            "feat (x): space before scope",
            "",
        ] {
            assert!(!is_conventional(s), "{s}");
        }
    }
}
