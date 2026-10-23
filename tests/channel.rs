//! The splice channel in `channel/` (spec D16), checked without splice: the manifest and pieces,
//! the classes of each file, and GitHub expressions kept out of the template syntax.

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
    for v in ["project", "owner", "bosun_version", "rust_path", "ts_path"] {
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
    assert_eq!(ids, ["core", "rust", "ts-pnpm"]);
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
fn hoist_requires_the_four_checks() {
    let hoist = toml_at(&channel().join("pieces/core/files/hoist.toml"));
    let required = strings(hoist["github"].get("required_checks"));
    assert_eq!(required, bosun::cli::pr::REQUIRED);
}
