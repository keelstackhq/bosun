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

/// The env value (comma-separated, empty entries dropped) is the explicit full override. Else the
/// defaults, then the extra entries of `hoist.toml`: a repo can add checks, never drop defaults.
pub fn required_checks(env: Option<&str>, hoist_toml: Option<&str>) -> Vec<String> {
    let from_env: Vec<String> = env
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if !from_env.is_empty() {
        return from_env;
    }
    let mut out: Vec<String> = DEFAULT_REQUIRED.iter().map(|s| s.to_string()).collect();
    for c in hoist_toml.and_then(hoist_required).unwrap_or_default() {
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

fn unquote(s: &str) -> &str {
    let s = s.split(" #").next().unwrap_or(s).trim();
    s.trim_matches(|c| c == '"' || c == '\'')
}

/// The check contexts of a workflow: for each key directly under the top-level `jobs:`, its
/// `name:` if it has one, else the key. Not a YAML parser; it reads the indentation.
pub fn workflow_checks(yaml: &str) -> Vec<String> {
    let mut jobs: Vec<(String, Option<String>)> = Vec::new();
    let (mut in_jobs, mut job_indent, mut name_indent) = (false, None::<usize>, None::<usize>);
    for line in yaml.lines() {
        let body = line.trim_start();
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        let indent = line.len() - body.len();
        if indent == 0 {
            in_jobs = unquote(body.trim_end()) == "jobs:" || body.starts_with("jobs: #");
            job_indent = None;
            continue;
        }
        if !in_jobs {
            continue;
        }
        let ji = *job_indent.get_or_insert(indent);
        if indent == ji {
            let key = body.split(" #").next().unwrap_or(body).trim_end();
            if let Some(k) = key.strip_suffix(':') {
                jobs.push((unquote(k).to_string(), None));
                name_indent = None;
            }
        } else if indent > ji && !body.starts_with('-') {
            let ni = *name_indent.get_or_insert(indent);
            if indent == ni
                && let Some(v) = body.strip_prefix("name:")
                && let Some(last) = jobs.last_mut()
                && last.1.is_none()
            {
                last.1 = Some(unquote(v).to_string());
            }
        }
    }
    jobs.into_iter().map(|(k, n)| n.unwrap_or(k)).collect()
}

/// The environments the jobs of a workflow deploy to: each job's `environment:`, a name, a
/// mapping's `name:` or a flow mapping's `name:`. `Err(job)` for a job whose environment is an
/// expression (`${{`) or names none, so doctor can say it cannot check it. Not a YAML parser.
pub fn workflow_environments(yaml: &str) -> Vec<Result<String, String>> {
    let mut out: Vec<Result<String, String>> = Vec::new();
    let mut push = |v: &str, job: &str| {
        let r = if v.is_empty() || v.contains("${{") {
            Err(job.to_string())
        } else {
            Ok(v.to_string())
        };
        if !out.contains(&r) {
            out.push(r);
        }
    };
    let (mut in_jobs, mut job_indent, mut key_indent) = (false, None::<usize>, None::<usize>);
    // The indent of an `environment:` mapping whose `name:` is still to come.
    let (mut open, mut job) = (None::<usize>, String::new());
    for line in yaml.lines() {
        let body = line.trim_start();
        if body.is_empty() || body.starts_with('#') {
            continue;
        }
        let indent = line.len() - body.len();
        if indent == 0 {
            in_jobs = body.split(" #").next().unwrap_or(body).trim_end() == "jobs:";
            (job_indent, key_indent, open) = (None, None, None);
            continue;
        }
        if !in_jobs {
            continue;
        }
        if indent <= *job_indent.get_or_insert(indent) {
            if open.take().is_some() {
                push("", &job);
            }
            key_indent = None;
            job = unquote(body).trim_end_matches(':').to_string();
            continue;
        }
        if open.is_some_and(|ei| indent > ei) {
            if let Some(v) = body.strip_prefix("name:") {
                push(unquote(v), &job);
                open = None;
            }
            continue;
        }
        if open.take().is_some() {
            push("", &job);
        }
        if indent == *key_indent.get_or_insert(indent)
            && let Some(v) = body.strip_prefix("environment:")
        {
            match unquote(v) {
                "" => open = Some(indent),
                v if v.starts_with('{') => {
                    let map = v.trim_start_matches('{').trim_end_matches('}');
                    let name = map
                        .split(',')
                        .find_map(|kv| kv.trim().strip_prefix("name:"));
                    push(name.map(unquote).unwrap_or(""), &job);
                }
                v => push(v, &job),
            }
        }
    }
    if open.is_some() {
        push("", &job);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_env_then_hoist_then_defaults() {
        let h = "[github]\nrequired_checks = [\"a\", \"b\"]\n";
        assert_eq!(required_checks(Some("x, ,y,"), Some(h)), ["x", "y"]);
        assert_eq!(required_checks(Some(" , "), None), DEFAULT_REQUIRED);
        let extra = required_checks(None, Some(h));
        assert_eq!(extra[..4], DEFAULT_REQUIRED);
        assert_eq!(extra[4..], ["a", "b"]);
        let subset = "[github]\nrequired_checks = [\"ci-ok\"]\n";
        assert_eq!(required_checks(None, Some(subset)), DEFAULT_REQUIRED);
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

    #[test]
    fn workflow_checks_reads_only_top_level_jobs() {
        let y = "on:\n  push:\n    branches: [main]\nname: x\njobs: # all\n  ci-ok: # agg\n    runs-on: x\n    steps:\n      - name: not a job name\n  \"quoted\":\n    name: Pretty\n    runs-on: x\n  other:\n    steps:\n      - run: a\nenv:\n  late:\n    x: 1\n";
        assert_eq!(workflow_checks(y), ["ci-ok", "Pretty", "other"]);
    }

    #[test]
    fn workflow_environments_reads_each_jobs_environment() {
        let y = "on:\n  workflow_dispatch:\n    inputs:\n      environment:\n        name: no\njobs:\n  a:\n    environment: release-pr # pr\n    steps:\n      - environment: no\n  b:\n    environment:\n      url: x\n      name: 'splice-bot'\n    env:\n      name: no\n  c:\n    environment: release-pr\n  d:\n    environment: ${{ inputs.e }}\n  e:\n    environment: {name: prod, url: x}\n  f:\n    environment: { url: x }\n  g:\n    environment:\n      name: ${{ inputs.e }}\n";
        let want = [
            Ok("release-pr"),
            Ok("splice-bot"),
            Err("d"),
            Ok("prod"),
            Err("f"),
            Err("g"),
        ];
        let want: Vec<Result<String, String>> = want
            .iter()
            .map(|r| r.map(String::from).map_err(String::from))
            .collect();
        assert_eq!(workflow_environments(y), want);
    }

    #[test]
    fn workflow_environments_finds_bosuns_own() {
        // Each file starts at indent 0, so the files read as one.
        let all = include_str!("../../.github/workflows/release.yml").to_owned()
            + include_str!("../../.github/workflows/review.yml")
            + include_str!("../../.github/workflows/splice.yml");
        let want = ["release-pr", "release", "review", "splice-bot"].map(|n| Ok(n.to_string()));
        assert_eq!(workflow_environments(&all), want);
    }
}
