//! `src/core` holds the process logic only; I/O lives in the command and client modules.

use std::fs;
use std::path::Path;

const FORBIDDEN: [&str; 7] = [
    "std::fs",
    "std::process",
    "std::net",
    "std::env",
    "crate::github",
    "crate::ctx",
    "ureq",
];

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn core_does_no_io() {
    let mut files = Vec::new();
    rust_files(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/core")
            .as_path(),
        &mut files,
    );
    assert!(!files.is_empty());
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        for (n, line) in text.lines().enumerate() {
            for bad in FORBIDDEN {
                assert!(
                    !line.contains(bad),
                    "{}:{}: `{bad}` in src/core; move the I/O to the command module",
                    file.display(),
                    n + 1
                );
            }
        }
    }
}
