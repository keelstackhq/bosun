//! R-deps reads every spelling of a pnpm catalog, and counts source changes (spec §8).
use bosun::core::risk::new_dependencies;

fn deps(path: &str, base: &str, head: &str) -> Vec<String> {
    new_dependencies(&[(path.into(), Some(base.into()), Some(head.into()))])
}

fn ws(base: &str, head: &str) -> Vec<String> {
    deps("pnpm-workspace.yaml", base, head)
}

#[test]
fn every_catalog_key_spelling_is_read() {
    for key in [
        "catalog:",
        "\"catalog\":",
        "'catalog':",
        "catalog :",
        "catalog: &c",
    ] {
        let base = format!("{key}\n  react: ^18\n");
        let head = format!("{key}\n  react: ^18\n  left-pad: 1.3.0\n");
        assert_eq!(ws(&base, &head), ["left-pad"], "{key}");
        assert!(ws(&base, &base).is_empty(), "{key}");
    }
    for key in ["catalogs:", "\"catalogs\":", "'catalogs' :"] {
        let base = format!("{key}\n  legacy:\n    react: ^17\n");
        let head = format!("{key}\n  legacy:\n    react: ^17\n  'next':\n    \"@x/y\": 1.0.0\n");
        assert_eq!(ws(&base, &head), ["@x/y"], "{key}");
    }
}

#[test]
fn what_the_reader_cannot_read_counts() {
    let base = "catalog:\n  react: ^18\n";
    for head in [
        "catalog: {react: ^18, evil: 1.0.0}\n",
        "catalog: *shared\n",
        "catalog:\n  react: ^18\n  <<: *more\n",
        "catalog:\n  react: ^18\n  evil: &e 1.0.0\n",
        "catalog:\n  react: ^18\n    deeper: 1.0.0\n",
        "catalogs:\n  evil: 1.0.0\n",
    ] {
        assert!(!ws(base, head).is_empty(), "{head}");
    }
}

#[test]
fn a_version_bump_is_not_new() {
    assert!(
        ws(
            "catalog:\n  react: ^18.2.0\n",
            "catalog:\n  react: ^18.3.1\n"
        )
        .is_empty()
    );
    let (b, h) = (
        r#"{"dependencies":{"a":"^1.0.0"}}"#,
        r#"{"dependencies":{"a":"~1.2.3"}}"#,
    );
    assert!(deps("package.json", b, h).is_empty());
    let (b, h) = (
        "[dependencies]\nx = \"1\"\n",
        "[dependencies]\nx = { version = \"2\" }\n",
    );
    assert!(deps("Cargo.toml", b, h).is_empty());
}

#[test]
fn cargo_source_changes_count() {
    let base = "[dependencies]\nx = \"1\"\n";
    for head in [
        "[dependencies]\nx = { git = \"https://e.test/x\" }\n",
        "[dependencies]\nx = { version = \"1\", package = \"evil\" }\n",
        "[dependencies]\nx = { version = \"1\", registry = \"other\" }\n",
    ] {
        assert_eq!(
            deps("Cargo.toml", base, head),
            ["x (source changed)"],
            "{head}"
        );
    }
    let patched =
        "[dependencies]\nx = \"1\"\n[patch.crates-io]\nx = { git = \"https://e.test/x\" }\n";
    assert_eq!(deps("Cargo.toml", base, patched), ["patch.crates-io/x"]);
    let replaced =
        "[dependencies]\nx = \"1\"\n[replace]\n\"x:1.0.0\" = { git = \"https://e.test/x\" }\n";
    assert_eq!(deps("Cargo.toml", base, replaced), ["replace/x:1.0.0"]);
    // A patch to in-repo code is in the diff.
    let local = "[dependencies]\nx = \"1\"\n[patch.crates-io]\nx = { path = \"vendor/x\" }\n";
    assert!(deps("Cargo.toml", base, local).is_empty());
}

#[test]
fn npm_source_changes_and_overrides_count() {
    let base = r#"{"dependencies":{"a":"^1"}}"#;
    for (head, want) in [
        (
            r#"{"dependencies":{"a":"npm:evil@1"}}"#,
            "a (source changed)",
        ),
        (
            r#"{"dependencies":{"a":"github:u/a"}}"#,
            "a (source changed)",
        ),
        (
            r#"{"dependencies":{"a":"^1"},"overrides":{"b":"1.0.0"}}"#,
            "overrides/b",
        ),
        (
            r#"{"dependencies":{"a":"^1"},"resolutions":{"b":"1.0.0"}}"#,
            "resolutions/b",
        ),
        (
            r#"{"dependencies":{"a":"^1"},"pnpm":{"overrides":{"b":"1"}}}"#,
            "overrides/b",
        ),
        (
            r#"{"dependencies":{"a":"^1"},"pnpm":{"patchedDependencies":{"a@1":"p.patch"}}}"#,
            "patchedDependencies/a@1",
        ),
    ] {
        assert_eq!(deps("package.json", base, head), [want], "{head}");
    }
    let (b, h) = (
        "overrides:\n  b: 1.0.0\n",
        "overrides:\n  b: npm:evil@1\npatchedDependencies:\n  a@1: patches/a.patch\n",
    );
    assert_eq!(
        ws(b, h),
        ["overrides/b (source changed)", "patchedDependencies/a@1"]
    );
}
