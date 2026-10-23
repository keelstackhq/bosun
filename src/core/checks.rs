//! Which checks must be green before auto-merge (spec D15). Pure: callers pass the env value
//! and the text of `hoist.toml`.

/// The defaults, used when neither the env nor `hoist.toml` names any.
pub const DEFAULT_REQUIRED: [&str; 4] = ["ci-ok", "risk-check", "review-verdict", "main-open"];

/// `[github] required_checks` of a `hoist.toml` text: a non-empty array of strings, else `None`.
pub fn hoist_required(hoist_toml: &str) -> Option<Vec<String>> {
    let doc: toml::Table = hoist_toml.parse().ok()?;
    let list: Vec<String> = doc
        .get("github")?
        .get("required_checks")?
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(String::from))
        .collect::<Option<_>>()?;
    (!list.is_empty()).then_some(list)
}

/// The env value (comma-separated) if non-empty, else `hoist.toml`, else the defaults.
pub fn required_checks(env: Option<&str>, hoist_toml: Option<&str>) -> Vec<String> {
    if let Some(v) = env.filter(|v| !v.trim().is_empty()) {
        return v.split(',').map(|s| s.trim().to_string()).collect();
    }
    hoist_toml
        .and_then(hoist_required)
        .unwrap_or_else(|| DEFAULT_REQUIRED.iter().map(|s| s.to_string()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_env_then_hoist_then_defaults() {
        let h = "[github]\nrequired_checks = [\"a\", \"b\"]\n";
        assert_eq!(required_checks(Some("x, y"), Some(h)), ["x", "y"]);
        assert_eq!(required_checks(Some(" "), Some(h)), ["a", "b"]);
        assert_eq!(required_checks(None, Some(h)), ["a", "b"]);
        for bad in [
            "[github]\nrequired_checks = []\n",
            "[github]\nrequired_checks = [1]\n",
            "x = [",
            "",
        ] {
            assert_eq!(required_checks(None, Some(bad)).len(), 4, "{bad}");
        }
        assert_eq!(required_checks(None, None).len(), 4);
    }
}
