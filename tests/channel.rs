//! The splice channel in `channel/` (spec D16), checked without splice: the manifest and pieces,
//! the classes of each file, GitHub expressions kept out of the template syntax, and workflows
//! that run the pinned bosun release, never code from the PR head.

use std::fs;
use std::path::{Path, PathBuf};

fn channel() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("channel")
}

fn toml_at(path: &Path) -> toml::Table {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.parse()
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files_under(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn strings(v: Option<&toml::Value>) -> Vec<String> {
    v.and_then(toml::Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn manifest_names_the_bosun_channel_with_core_as_base() {
    let m = toml_at(&channel().join("splice.toml"));
    assert_eq!(m["format"].as_integer(), Some(1));
    assert_eq!(m["name"].as_str(), Some("bosun"));
    assert_eq!(m["base"].as_str(), Some("core"));
    // Sections and raw blocks need splice 0.2.0 (template format §9.23, §9.26).
    assert_eq!(m["min_splice"].as_str(), Some("0.2.0"));
    let vars = m["vars"].as_table().unwrap();
    for v in [
        "project",
        "owner",
        "bosun_version",
        "rust_path",
        "ts_path",
        "own_ci",
    ] {
        assert!(vars.contains_key(v), "splice.toml lacks var {v}");
    }
}

#[test]
fn each_piece_matches_its_directory_and_lists_real_files() {
    let mut ids: Vec<String> = fs::read_dir(channel().join("pieces"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    ids.sort();
    assert_eq!(ids, ["claude-md", "core", "rust", "ts-pnpm"]);
    for id in &ids {
        let dir = channel().join("pieces").join(id);
        let p = toml_at(&dir.join("piece.toml"));
        assert_eq!(p["id"].as_str(), Some(id.as_str()));
        if id != "core" {
            assert_eq!(strings(p.get("requires")), ["core"], "{id} requires core");
        }
        let files = p.get("files").and_then(toml::Value::as_table);
        for class in ["owned", "shared", "seed"] {
            for f in strings(files.and_then(|t| t.get(class))) {
                let plain = dir.join("files").join(&f);
                let jinja = dir.join("files").join(format!("{f}.jinja"));
                assert!(
                    plain.is_file() || jinja.is_file(),
                    "{id}: {class} {f} has no file"
                );
            }
        }
        if let Some(sections) = p.get("sections").and_then(toml::Value::as_table) {
            for (host, fragment) in sections {
                let f = dir.join("files").join(fragment.as_str().unwrap());
                assert!(f.is_file(), "{id}: section {host} has no fragment");
            }
        }
    }
}

/// `${{ }}` of GitHub Actions is template syntax too; in a `.jinja` file it must sit inside a raw
/// block, and a plain file must not contain template tags at all.
#[test]
fn github_expressions_stay_out_of_the_template_syntax() {
    let mut all = Vec::new();
    files_under(&channel().join("pieces"), &mut all);
    for path in all {
        let text = fs::read_to_string(&path).unwrap();
        let name = path.display();
        if path.extension().is_some_and(|e| e == "jinja") {
            let mut outside = String::new();
            let mut rest = text.as_str();
            while let Some(i) = rest.find("{% raw %}") {
                outside.push_str(&rest[..i]);
                let end = rest[i..]
                    .find("{% endraw %}")
                    .expect("unterminated raw block");
                rest = &rest[i + end + "{% endraw %}".len()..];
            }
            outside.push_str(rest);
            assert!(
                !outside.contains("${{"),
                "{name}: `${{{{` outside a raw block"
            );
        } else {
            let no_gh = text.replace("${{", "");
            assert!(
                !no_gh.contains("{{") && !no_gh.contains("{%"),
                "{name}: template tags in a file without the .jinja suffix"
            );
        }
    }
}

#[test]
fn hoist_requires_the_four_checks_and_bosun_ok_beside_a_repo_ci() {
    let path = channel().join("pieces/core/files/hoist.toml.jinja");
    let text = fs::read_to_string(path).unwrap();
    let beside = r#"{% if own_ci %}"bosun-ok", {% endif %}"#;
    assert!(text.contains(beside), "{text}");
    let hoist: toml::Table = text.replace(beside, "").parse().unwrap();
    let required = strings(hoist["github"].get("required_checks"));
    assert_eq!(required, bosun::cli::pr::REQUIRED);
}

#[test]
fn risk_and_verdict_run_the_pinned_release_not_pr_code() {
    let wf = channel().join("pieces/core/files/.github/workflows");
    for f in ["risk.yml", "verdict.yml"] {
        let y = fs::read_to_string(wf.join(f)).unwrap();
        assert!(!y.contains("cargo build"), "{f} builds bosun");
        assert!(
            y.contains(r#"npm install -g --ignore-scripts "@keelstackhq/bosun@${v}""#),
            "{f} does not install the pinned release"
        );
        assert!(
            !y.contains("head.sha") && !y.contains("head.ref"),
            "{f} checks out the PR head"
        );
    }
    let risk = fs::read_to_string(wf.join("risk.yml")).unwrap();
    assert!(risk.contains("ref: ${{ github.event.pull_request.base.sha || github.sha }}"));
    // An owner's review is read on the next run; `/bosun recheck` starts one without a push, and
    // no review event runs the PR's copy of the file (tests/workflows.rs scans every workflow).
    let recheck = "contains(github.event.comment.body, '/bosun recheck')";
    assert_eq!(risk.matches(recheck).count(), 2, "job if: and concurrency");
    assert!(!risk.contains("\n  pull_request_review:"), "{risk}");
}

#[test]
fn one_job_carries_the_required_check() {
    let wf = channel().join("pieces/core/files/.github/workflows");
    // bosun.yml, never ci.yml: a repo's own ci.yml (a hull template) stays untouched.
    assert!(!wf.join("ci.yml.jinja").exists() && !wf.join("ci.yml").exists());
    let ci = fs::read_to_string(wf.join("bosun.yml.jinja")).unwrap();
    let body = &ci[ci.find("\njobs:\n").unwrap()..];
    // CI minutes: every job is rounded up to a whole minute, so a rust stack has exactly one
    // job, named after the required check: ci-ok, or bosun-ok beside the repo's own ci-ok. A
    // ts-pnpm stack has one more, bosun-trusted, that runs no PR code (#234).
    let ids: Vec<_> = body
        .lines()
        .filter(|l| l.starts_with("  ") && !l.starts_with("   ") && l.ends_with(':'))
        .collect();
    assert_eq!(
        ids,
        ["  bosun-trusted:"],
        "every other job id sits in the own_ci switch"
    );
    let ts = "{% if \"ts-pnpm\" in pieces %}";
    assert!(body.find(ts).unwrap() < body.find("  bosun-trusted:").unwrap());
    let head = "{% if own_ci %}  bosun-ok:\n{% else %}  ci-ok:\n{% endif %}";
    assert_eq!(
        body.matches(head).count(),
        2,
        "the required check: the rust job, or the ts-pnpm job after bosun-trusted"
    );
    assert_eq!(body.matches("needs:").count(), 1);
    assert!(body.contains("    needs: bosun-trusted\n"));
    assert!(!body.contains("upload-artifact"));
    // A draft fails it at once (never skipped: a skipped required check counts as passed);
    // marking a PR ready runs it.
    assert!(!body.contains("pull_request.draft\n"));
    assert!(body.contains("DRAFT: ${{ github.event.pull_request.draft || false }}"));
    // The gates that run no PR code first (lint and test run build.rs and tests).
    assert!(body.contains("GATES: test-integrity deps fmt lint test\n"));
    assert!(ci.contains("types: [opened, synchronize, reopened, edited, ready_for_review]"));
    // The first failed gate stops the job: no PR code (lint, test) runs after a failure.
    assert!(body.contains(
        "\"$bosun\" gate \"$g\" $all || { echo \"::error::gate $g failed; later gates did not run\"; exit 1; }"
    ));
    assert!(!body.contains("failed=\""));
}

/// The `run:` scripts of a workflow: the `run:` line and its indented block.
fn run_scripts(yaml: &str) -> Vec<String> {
    let lines: Vec<&str> = yaml.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("run:").or_else(|| t.strip_prefix("- run:")) else {
            continue;
        };
        let indent = line.len() - t.len();
        let mut script = rest.to_string();
        for next in &lines[i + 1..] {
            if !next.trim().is_empty() && next.len() - next.trim_start().len() <= indent {
                break;
            }
            script.push('\n');
            script.push_str(next);
        }
        out.push(script);
    }
    out
}

/// The text from `start` up to `end` (exclusive).
fn block<'a>(yaml: &'a str, start: &str, end: &str) -> &'a str {
    let i = yaml.find(start).unwrap_or_else(|| panic!("no {start:?}"));
    let j = yaml[i..].find(end).unwrap_or_else(|| panic!("no {end:?}")) + i;
    &yaml[i..j]
}

/// main-health's own_ci parts, as written in the template.
const WATCH: &str = "{% endraw %}    workflows: [bosun{% if own_ci %}, ci{% endif %}]\n{% raw %}";
const ACTIONS_READ: &str = "{% endraw %}{% if own_ci %}  actions: read\n{% endif %}{% raw %}";

/// The consumer copies keep bosun's own triggers and permissions (tests/workflows.rs pins
/// those), check out at most the base once, and never put an expression in a script.
#[test]
fn consumer_workflows_keep_bosuns_triggers_permissions_and_env_only_scripts() {
    let ours = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows");
    let wf = channel().join("pieces/core/files/.github/workflows");
    for (f, theirs_name) in [
        ("risk.yml", "risk.yml"),
        ("verdict.yml", "verdict.yml"),
        ("main-open.yml", "main-open.yml"),
        ("main-health.yml", "main-health.yml.jinja"),
    ] {
        let theirs = fs::read_to_string(wf.join(theirs_name)).unwrap();
        let mine = fs::read_to_string(ours.join(f)).unwrap();
        // main-health watches bosun (and the repo's ci) where bosun's own watches ci, and with
        // the repo's ci it also reads the runs of both: the rest of `on:` and `permissions:` match.
        let theirs = theirs
            .replace(WATCH, "    workflows: [ci]\n")
            .replace(ACTIONS_READ, "");
        for (a, b) in [
            ("\non:\n", "\npermissions:"),
            ("\npermissions:\n", "\njobs:"),
        ] {
            assert_eq!(
                block(&theirs, a, b),
                block(&mine, a, b),
                "{f}: {a:?} differs"
            );
        }
        assert_eq!(
            theirs.matches("permissions:").count(),
            1,
            "{f}: job-level permissions"
        );
    }
    for f in ["risk.yml", "verdict.yml"] {
        let y = fs::read_to_string(wf.join(f)).unwrap();
        assert_eq!(y.matches("actions/checkout@").count(), 1, "{f}");
        assert!(y.contains("persist-credentials: false"), "{f}");
    }
    let mut all = Vec::new();
    files_under(&wf, &mut all);
    for path in all {
        let y = fs::read_to_string(&path).unwrap();
        let scripts = run_scripts(&y);
        for s in scripts {
            assert!(
                !s.contains("${{"),
                "{}: expression in a script: {s}",
                path.display()
            );
        }
    }
    // A fork PR from a branch called main also fires workflow_run on [main].
    let health = fs::read_to_string(wf.join("main-health.yml.jinja")).unwrap();
    assert!(health.contains("uses: ./.github/workflows/bosun.yml"));
    assert!(health.contains(WATCH) && health.contains(ACTIONS_READ));
    // With the repo's ci, a green run of one workflow alone never closes main-red.
    let both = "for wf in bosun.yml ci.yml; do";
    let gate = health.find(both).expect("own_ci checks both workflows");
    assert!(gate < health.find("gh issue close").unwrap());
    assert!(health[..gate].rfind("{% if own_ci %}") > health[..gate].rfind("{% endif %}"));
    // Both report paths (bosun main-health and the shell fallback) read that judged result.
    let judged = "RESULT: {% endraw %}{% if own_ci %}{% raw %}${{ steps.result.outputs.result }}";
    assert_eq!(health.matches(judged).count(), 2, "both report paths");
    assert!(health.find(judged).unwrap() > gate);
    assert!(health.contains("github.event.workflow_run.event == 'push'"));
    assert!(health.contains("workflow_run.head_repository.full_name == github.repository"));
    // In a consumer repo the pinned-release steps are in bosun.yml; ci.yml is the repo's own.
    for f in ["main-open.yml", "main-health.yml.jinja"] {
        let y = fs::read_to_string(wf.join(f)).unwrap();
        assert!(y.contains("pinned-release steps as bosun.yml"), "{f}");
        assert!(!y.contains("job of ci.yml"), "{f}");
    }
    // The update bot can write: its splice is pinned.
    let bot = fs::read_to_string(wf.join("splice.yml")).unwrap();
    let script =
        fs::read_to_string(channel().join("pieces/core/files/docs/setup-script.sh.jinja")).unwrap();
    let pinned = script
        .lines()
        .find_map(|l| l.strip_prefix("SPLICE_VERSION="))
        .expect("the setup script pins SPLICE_VERSION");
    assert_eq!(pinned, "0.2.1");
    assert!(
        bot.contains(&format!("version: {pinned}\n")),
        "splice.yml moves with the setup script's SPLICE_VERSION"
    );
    // Its token lives in environment splice-bot (main only): a PR's own workflow cannot read it.
    assert!(
        bot.contains("  update:\n    environment: splice-bot\n"),
        "{bot}"
    );
    assert!(bot.contains("secrets.SPLICE_BOT_TOKEN"));
}

/// Every occurrence of `script` in `text` is a call through bash: `bash <script>`.
fn calls_without_bash(text: &str, script: &str) -> usize {
    text.match_indices(script)
        .filter(|(i, _)| !text[..*i].ends_with("bash "))
        .count()
}

/// Splice does not carry file modes (template format 9.13): a channel script lands as 0644, so
/// the channel's docs, sections and workflows (and bosun's own docs, for its copy in scripts/)
/// call each shipped script through bash, never directly.
#[test]
fn shipped_scripts_are_only_ever_called_through_bash() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut channel_files = Vec::new();
    files_under(&channel().join("pieces"), &mut channel_files);
    // A script is a file the channel ships that ends in .sh. docs/setup-script.sh is not run in a
    // repo: the owner pastes it into the cloud environment's settings.
    let mut scripts: Vec<String> = channel_files
        .iter()
        .filter_map(|p| {
            let rel = p.to_str()?.split("/files/").nth(1)?.to_string();
            (rel.ends_with(".sh") && rel != "docs/setup-script.sh").then_some(rel)
        })
        .collect();
    scripts.push("scripts/fold-fragments.sh".into());
    assert!(
        scripts.contains(&".bosun/fold-fragments.sh".to_string()),
        "{scripts:?}"
    );

    let mut texts: Vec<PathBuf> = channel_files
        .into_iter()
        .filter(|p| {
            p.to_str()
                .is_some_and(|s| s.contains("/files/") && !s.ends_with(".sh"))
        })
        .collect();
    for f in [
        "AGENTS.md",
        "CLAUDE.md",
        "README.md",
        "docs/releasing.md",
        "docs/spec.md",
        "changelog.d/README.md",
        "docs/progress.d/README.md",
    ] {
        texts.push(root.join(f));
    }
    files_under(&root.join("docs/spec"), &mut texts);
    files_under(&root.join(".github/workflows"), &mut texts);
    let mut found = 0;
    for path in &texts {
        let text = fs::read_to_string(path).unwrap();
        for s in &scripts {
            found += text.matches(s.as_str()).count();
            assert_eq!(
                calls_without_bash(&text, s),
                0,
                "{} calls {s} without `bash ` (splice ships it as 0644)",
                path.display()
            );
        }
    }
    assert!(found >= 2, "the fold script is documented for both copies");
}
