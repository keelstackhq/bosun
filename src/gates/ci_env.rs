//! CI context from the environment (spec §7): the provider, the event and how to find the base.
//! Pure: the caller passes an env lookup and runs any git command itself.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    GitHub,
    CircleCi,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    Pr,
    DefaultPush,
    MergeQueue,
    Unknown,
}

/// How to find the base. `Legacy` keeps the pre-CircleCI rules (`GITHUB_BASE_REF`, `origin/HEAD`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseRule {
    Legacy,
    /// Use this ref or sha as is.
    Ref(String),
    /// `git merge-base HEAD <ref>`.
    MergeBase(String),
    /// Named by a merge-queue branch: untrusted until the caller checks that `base` is the
    /// default branch and `sha` is on it.
    Queue {
        sha: String,
        base: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CiContext {
    pub provider: Provider,
    pub event: Event,
    pub base: BaseRule,
    /// Run for the whole repo: no diff is needed.
    pub all: bool,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Provider::GitHub => "github",
            Provider::CircleCi => "circleci",
            Provider::Local => "local",
        }
    }
}

impl Event {
    pub fn name(self) -> &'static str {
        match self {
            Event::Pr => "pr",
            Event::DefaultPush => "push",
            Event::MergeQueue => "merge-queue",
            Event::Unknown => "unknown",
        }
    }
}

/// `(base branch, base sha)` of `gh-readonly-queue/<base>/pr-<n>-<sha>`. The base may contain `/`.
fn merge_queue(branch: &str) -> Option<(&str, &str)> {
    let (base, last) = branch
        .strip_prefix("gh-readonly-queue/")?
        .rsplit_once('/')?;
    let (n, sha) = last.strip_prefix("pr-")?.split_once('-')?;
    let digits = |s: &str, radix| !s.is_empty() && s.chars().all(|c| c.is_digit(radix));
    (digits(n, 10) && digits(sha, 16)).then_some((base, sha))
}

/// `default_branch` is the repo's default branch name (`main`).
pub fn detect(env: &dyn Fn(&str) -> Option<String>, default_branch: &str) -> CiContext {
    let get = |k: &str| {
        env(k)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let is_true = |k: &str| get(k).is_some_and(|v| v == "true");
    let legacy = BaseRule::Legacy;
    let (provider, event, base, all) = if is_true("CIRCLECI") {
        let branch = get("CIRCLE_BRANCH").unwrap_or_default();
        let default_ref = format!("origin/{default_branch}");
        // The branch name is author-controlled: a pull request is never a queue or a push.
        let is_pr = get("CIRCLE_PULL_REQUEST")
            .or_else(|| get("CIRCLE_PR_NUMBER"))
            .is_some();
        let (event, base, all) = if is_pr {
            (Event::Pr, BaseRule::MergeBase(default_ref), false)
        } else if let Some((qbase, sha)) = merge_queue(&branch) {
            let (sha, base) = (sha.to_string(), qbase.to_string());
            (Event::MergeQueue, BaseRule::Queue { sha, base }, true)
        } else if branch == default_branch {
            (Event::DefaultPush, BaseRule::Ref(default_ref), true)
        } else if branch.is_empty() {
            // Tag builds and API pipelines have no branch: run the whole repo (fail closed).
            (Event::Unknown, BaseRule::Ref(default_ref), true)
        } else {
            (Event::Pr, BaseRule::MergeBase(default_ref), false)
        };
        (Provider::CircleCi, event, base, all)
    } else if is_true("GITHUB_ACTIONS") {
        let event = match get("GITHUB_EVENT_NAME").as_deref() {
            Some("pull_request" | "pull_request_target") => Event::Pr,
            Some("merge_group") => Event::MergeQueue,
            Some("push") => Event::DefaultPush,
            _ => Event::Unknown,
        };
        (Provider::GitHub, event, legacy, false)
    } else {
        (Provider::Local, Event::Unknown, legacy, false)
    };
    CiContext {
        provider,
        event,
        base,
        all,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `spec` is a space-separated `KEY=value` list; the result reads `provider event base all` (`mb` = merge-base).
    fn show(spec: &str) -> String {
        let env = |k: &str| {
            spec.split_whitespace()
                .filter_map(|kv| kv.split_once('='))
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        };
        let c = detect(&env, "main");
        let base = match c.base {
            BaseRule::Legacy => "legacy".to_string(),
            BaseRule::Ref(r) => format!("ref:{}", r.trim_start_matches("origin/")),
            BaseRule::MergeBase(r) => format!("mb:{}", r.trim_start_matches("origin/")),
            BaseRule::Queue { sha, base } => format!("queue:{base}@{sha}"),
        };
        format!("{} {} {base} {}", c.provider.name(), c.event.name(), c.all)
    }

    #[test]
    fn circleci_branches() {
        let rows = [
            ("feat/x", "pr mb:main false"),
            ("main", "push ref:main true"),
            (
                "gh-readonly-queue/main/pr-7-0123ab",
                "mq queue:main@0123ab true",
            ),
            (
                "gh-readonly-queue/rel/1.0/pr-12-ab12",
                "mq queue:rel/1.0@ab12 true",
            ),
            // malformed queue branches are ordinary branches
            ("gh-readonly-queue/main/pr-x-abc", "pr mb:main false"),
            ("gh-readonly-queue/main/pr-7-zz", "pr mb:main false"),
            ("gh-readonly-queue/pr-7-ab", "pr mb:main false"),
            ("", "unknown ref:main true"),
        ];
        for (branch, want) in rows {
            let got = show(&format!("CIRCLECI=true CIRCLE_BRANCH={branch}"));
            let want = want.replacen("mq", "merge-queue", 1);
            assert_eq!(got, format!("circleci {want}"), "{branch}");
        }
    }

    #[test]
    fn a_pull_request_is_never_a_queue_or_a_push() {
        for pr in ["CIRCLE_PULL_REQUEST=https://x/pull/1", "CIRCLE_PR_NUMBER=5"] {
            for b in ["gh-readonly-queue/main/pr-1-ab", "main", ""] {
                let got = show(&format!("CIRCLECI=true CIRCLE_BRANCH={b} {pr}"));
                assert_eq!(got, "circleci pr mb:main false", "{b} {pr}");
            }
        }
    }

    #[test]
    fn other_providers_keep_the_legacy_rules() {
        for (ev, want) in [("pull_request", "pr"), ("push", "push")] {
            let got = show(&format!("GITHUB_ACTIONS=true GITHUB_EVENT_NAME={ev}"));
            assert_eq!(got, format!("github {want} legacy false"), "{ev}");
        }
        assert_eq!(show(""), "local unknown legacy false");
        assert_eq!(show("CIRCLECI=false"), "local unknown legacy false");
    }
}
