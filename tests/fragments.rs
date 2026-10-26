//! CHANGELOG and PROGRESS fragments: a PR adds `changelog.d/<branch>.md` or
//! `docs/progress.d/<branch>.md` (one line each) instead of appending to the shared files, and
//! `scripts/fold-fragments.sh` folds them in a docs PR to main before a release. Two PRs then never conflict there.

use std::fs;
use std::path::Path;
use std::process::Command;

const SCRIPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/fold-fragments.sh");

const CHANGELOG: &str =
    "# Changelog\n\nIntro.\n\n## Unreleased\n\n## 0.2.1 (2026-10-26)\n\n- old\n";
const PROGRESS: &str =
    "# Progress\n\n## Status\n\nok\n\n## Done\n\n- d1\n- d2\n\n## Blocked\n\n1. b\n";

/// A repo with the two files and the given fragments `(path, text)`.
fn repo(frags: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("changelog.d")).unwrap();
    fs::create_dir_all(dir.path().join("docs/progress.d")).unwrap();
    fs::write(dir.path().join("CHANGELOG.md"), CHANGELOG).unwrap();
    fs::write(dir.path().join("docs/PROGRESS.md"), PROGRESS).unwrap();
    fs::write(
        dir.path().join("changelog.d/README.md"),
        "# not a fragment\n\nmore\n",
    )
    .unwrap();
    for (p, t) in frags {
        fs::write(dir.path().join(p), t).unwrap();
    }
    dir
}

fn fold(dir: &Path) -> bool {
    Command::new("bash")
        .arg(SCRIPT)
        .arg(dir)
        .output()
        .unwrap()
        .status
        .success()
}

fn read(dir: &Path, p: &str) -> String {
    fs::read_to_string(dir.join(p)).unwrap()
}

#[test]
fn fragments_fold_in_name_order_at_the_end_of_their_section() {
    let dir = repo(&[
        ("changelog.d/b-two.md", "- two\n"),
        ("changelog.d/a-one.md", "- one"),
        ("docs/progress.d/x.md", "- 2026-10-26 · T · done · PR #1\n"),
    ]);
    assert!(fold(dir.path()));
    assert_eq!(
        read(dir.path(), "CHANGELOG.md"),
        "# Changelog\n\nIntro.\n\n## Unreleased\n\n- one\n- two\n\n## 0.2.1 (2026-10-26)\n\n- old\n"
    );
    assert_eq!(
        read(dir.path(), "docs/PROGRESS.md"),
        "# Progress\n\n## Status\n\nok\n\n## Done\n\n- d1\n- d2\n- 2026-10-26 · T · done · PR #1\n\n## Blocked\n\n1. b\n"
    );
    // Folded fragments are gone; the README stays.
    let left: Vec<_> = fs::read_dir(dir.path().join("changelog.d"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(left, ["README.md"]);
    assert!(
        fs::read_dir(dir.path().join("docs/progress.d"))
            .unwrap()
            .next()
            .is_none()
    );
    // A second run has nothing to fold and changes nothing.
    let before = read(dir.path(), "CHANGELOG.md");
    assert!(fold(dir.path()));
    assert_eq!(read(dir.path(), "CHANGELOG.md"), before);
}

/// A very long line goes through: no pipe or environment size limit on the way.
#[test]
fn a_long_fragment_folds() {
    let long = format!("- {}\n", "x".repeat(200_000));
    let dir = repo(&[("changelog.d/a.md", &long)]);
    assert!(fold(dir.path()));
    assert!(read(dir.path(), "CHANGELOG.md").contains(&long));
    assert_eq!(
        fs::read_dir(dir.path()).unwrap().count(),
        3,
        "no temp file is left"
    );
}

#[test]
fn without_fragments_nothing_changes() {
    let dir = repo(&[]);
    assert!(fold(dir.path()));
    assert_eq!(read(dir.path(), "CHANGELOG.md"), CHANGELOG);
    assert_eq!(read(dir.path(), "docs/PROGRESS.md"), PROGRESS);
}

#[test]
fn a_bad_fragment_stops_the_fold_before_it_writes() {
    for bad in [
        "- a\n- b\n",
        "no dash\n",
        "",
        "-x\n",
        "- \n",
        "- crlf\r\n",
        "- a\n\n",
    ] {
        let dir = repo(&[
            ("changelog.d/a.md", "- good\n"),
            ("docs/progress.d/z.md", bad),
        ]);
        assert!(!fold(dir.path()), "{bad:?} folded");
        assert_eq!(read(dir.path(), "CHANGELOG.md"), CHANGELOG, "{bad:?}");
        assert!(dir.path().join("changelog.d/a.md").exists(), "{bad:?}");
    }
}

/// Both files or neither: a problem with the second target leaves the first unchanged.
#[test]
fn a_bad_target_writes_neither_file() {
    let frags = [
        ("changelog.d/a.md", "- good\n"),
        ("docs/progress.d/p.md", "- 2026 · T · x\n"),
    ];
    let no_done = "# Progress\n\n## Status\n\nok\n";
    let twice = "## Done\n\n- a\n\n## Done\n\n- b\n";
    for progress in [Some(no_done), Some(twice), None] {
        let dir = repo(&frags);
        match progress {
            Some(t) => fs::write(dir.path().join("docs/PROGRESS.md"), t).unwrap(),
            None => fs::remove_file(dir.path().join("docs/PROGRESS.md")).unwrap(),
        }
        assert!(!fold(dir.path()), "{progress:?}");
        assert_eq!(read(dir.path(), "CHANGELOG.md"), CHANGELOG, "{progress:?}");
        assert!(dir.path().join("changelog.d/a.md").exists());
        assert!(dir.path().join("docs/progress.d/p.md").exists());
        // No temp file is left behind.
        let left = fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(left, 3, "CHANGELOG.md, changelog.d, docs");
    }
}

/// A file the script would not fold is an error, never skipped without a word.
#[test]
fn anything_but_a_fragment_in_the_directories_is_an_error() {
    for (path, text) in [
        ("changelog.d/claude/foo.md", "- x\n"),
        ("changelog.d/a.txt", "- x\n"),
    ] {
        let dir = repo(&[("changelog.d/b.md", "- good\n")]);
        fs::create_dir_all(dir.path().join(path).parent().unwrap()).unwrap();
        fs::write(dir.path().join(path), text).unwrap();
        assert!(!fold(dir.path()), "{path}");
        assert_eq!(read(dir.path(), "CHANGELOG.md"), CHANGELOG, "{path}");
    }
}

/// A name with a tab, newline or space is refused before any write (review of #237: a tab-split
/// list made `x<TAB>Cargo.toml<TAB>y.md` delete the root Cargo.toml).
#[test]
fn a_fragment_name_with_odd_characters_is_refused_and_nothing_is_deleted() {
    for name in [
        "x\tCargo.toml\ty.md",
        "x\nCargo.toml\ny.md",
        "x Cargo.toml y.md",
        "-rf.md\u{e9}",
        "\u{e9}.md",
    ] {
        let dir = repo(&[("changelog.d/a.md", "- good\n")]);
        fs::write(dir.path().join("Cargo.toml"), "keep\n").unwrap();
        fs::write(dir.path().join("changelog.d").join(name), "- x\n").unwrap();
        assert!(!fold(dir.path()), "{name:?} folded");
        assert_eq!(read(dir.path(), "Cargo.toml"), "keep\n", "{name:?}");
        assert_eq!(read(dir.path(), "CHANGELOG.md"), CHANGELOG, "{name:?}");
        assert!(dir.path().join("changelog.d/a.md").exists(), "{name:?}");
        assert!(
            dir.path().join("changelog.d").join(name).exists(),
            "{name:?}"
        );
    }
    // Every allowed character folds, and only the fragments go.
    let dir = repo(&[("changelog.d/A-z_0.9.md", "- ok\n")]);
    fs::write(dir.path().join("Cargo.toml"), "keep\n").unwrap();
    assert!(fold(dir.path()));
    assert!(read(dir.path(), "CHANGELOG.md").contains("- ok\n"));
    assert_eq!(read(dir.path(), "Cargo.toml"), "keep\n");
    assert!(!dir.path().join("changelog.d/A-z_0.9.md").exists());
}

/// A symlinked target, fragment directory or fragment would let the fold write or delete
/// outside the repo (review of #237): each one is refused before any write.
#[test]
fn symlinks_are_refused() {
    use std::os::unix::fs::symlink;
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.md");
    // Both headings, so only the symlink rule can stop a fold into it.
    let body = format!("{CHANGELOG}\n## Done\n\n- d\n");
    fs::write(&victim, &body).unwrap();
    let cases: [&dyn Fn(&Path); 4] = [
        &|d| {
            fs::remove_file(d.join("CHANGELOG.md")).unwrap();
            symlink(&victim, d.join("CHANGELOG.md")).unwrap();
        },
        &|d| {
            fs::remove_file(d.join("docs/PROGRESS.md")).unwrap();
            symlink(&victim, d.join("docs/PROGRESS.md")).unwrap();
            fs::write(d.join("docs/progress.d/p.md"), "- p\n").unwrap();
        },
        &|d| {
            let real = outside.path().join("frags");
            fs::create_dir_all(&real).unwrap();
            fs::write(real.join("b.md"), "- b\n").unwrap();
            fs::remove_dir_all(d.join("docs/progress.d")).unwrap();
            symlink(&real, d.join("docs/progress.d")).unwrap();
        },
        &|d| symlink(&victim, d.join("changelog.d/link.md")).unwrap(),
    ];
    for (i, setup) in cases.iter().enumerate() {
        let dir = repo(&[("changelog.d/a.md", "- good\n")]);
        setup(dir.path());
        assert!(!fold(dir.path()), "case {i} folded");
        assert_eq!(fs::read_to_string(&victim).unwrap(), body, "case {i}");
        assert!(dir.path().join("changelog.d/a.md").exists(), "case {i}");
    }
}

/// README.md is the directory's README, never a fragment: one that reads like a fragment fails
/// loudly instead of being skipped with its entry lost (review of #237).
#[test]
fn a_readme_that_is_not_a_readme_fails() {
    for text in ["- an entry\n", "", "#no space\n"] {
        let dir = repo(&[("docs/progress.d/README.md", text)]);
        assert!(!fold(dir.path()), "{text:?} was skipped");
        assert_eq!(read(dir.path(), "docs/PROGRESS.md"), PROGRESS, "{text:?}");
    }
    let dir = repo(&[("docs/progress.d/README.md", "# progress.d\n\nrules\n")]);
    assert!(fold(dir.path()));
    assert!(dir.path().join("docs/progress.d/README.md").exists());
}

#[test]
fn the_fold_keeps_layout_mode_and_finds_the_repo_root() {
    // hoist writes "## <version>" right below "## Unreleased": a blank line goes before it.
    let dir = repo(&[("changelog.d/a.md", "- a\n")]);
    fs::write(
        dir.path().join("CHANGELOG.md"),
        "## Unreleased\n## 1.0.0\n- old\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    let target = dir.path().join("CHANGELOG.md");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(fold(dir.path()));
    assert_eq!(
        read(dir.path(), "CHANGELOG.md"),
        "## Unreleased\n\n- a\n\n## 1.0.0\n- old\n"
    );
    let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o644, "the target keeps its mode");
    // A section at the end of the file.
    let dir = repo(&[("docs/progress.d/a.md", "- new\n")]);
    fs::write(dir.path().join("docs/PROGRESS.md"), "## Done\n\n- old\n").unwrap();
    assert!(fold(dir.path()));
    assert_eq!(
        read(dir.path(), "docs/PROGRESS.md"),
        "## Done\n\n- old\n- new\n"
    );
    // Run from a subdirectory without an argument: it folds at the git top level.
    let dir = repo(&[("changelog.d/a.md", "- a\n")]);
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(dir.path())
                .output()
                .unwrap()
                .status
                .success()
        );
    };
    git(&["init", "-q"]);
    let ok = Command::new("bash")
        .arg(SCRIPT)
        .current_dir(dir.path().join("docs"))
        .status()
        .unwrap()
        .success();
    assert!(ok);
    assert!(read(dir.path(), "CHANGELOG.md").contains("## Unreleased\n\n- a\n"));
}

#[test]
fn the_channel_ships_the_same_script_and_the_repos_fragments_are_valid() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert_eq!(
        fs::read_to_string(SCRIPT).unwrap(),
        fs::read_to_string(root.join("channel/pieces/core/files/.bosun/fold-fragments.sh"))
            .unwrap(),
        "the channel's .bosun/fold-fragments.sh differs from scripts/fold-fragments.sh"
    );
    for d in ["changelog.d", "docs/progress.d"] {
        for e in fs::read_dir(root.join(d)).unwrap() {
            let p = e.unwrap().path();
            if p.file_name().unwrap() == "README.md" {
                continue;
            }
            // The script's rule: a regular *.md file, one line "- <text>", LF only.
            assert!(
                p.is_file() && p.extension().is_some_and(|e| e == "md"),
                "{}",
                p.display()
            );
            let t = fs::read_to_string(&p).unwrap();
            let line = t.strip_suffix('\n').unwrap_or(&t);
            assert!(
                line.starts_with("- ")
                    && !line[2..].starts_with(char::is_whitespace)
                    && !line.contains('\n')
                    && !line.contains('\r'),
                "{} must be exactly one line '- <text>'",
                p.display()
            );
        }
    }
}
