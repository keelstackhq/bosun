//! `bosun init` keeps a repo's own copy of every channel seed: its exclude list must name each
//! `seed` path of the channel pieces, or init would let splice overwrite a file the repo owns.

use std::path::Path;

/// The `seed` paths of every channel piece.
fn channel_seeds() -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("channel/pieces");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&root).expect("channel/pieces") {
        let piece = entry.expect("entry").path().join("piece.toml");
        let Ok(text) = std::fs::read_to_string(&piece) else {
            continue;
        };
        let doc: toml::Table = text.parse().expect("piece.toml");
        let seeds = doc
            .get("files")
            .and_then(|f| f.get("seed"))
            .or_else(|| doc.get("seed"));
        for s in seeds.and_then(|s| s.as_array()).into_iter().flatten() {
            out.push(s.as_str().expect("seed path").to_string());
        }
    }
    out.sort();
    out
}

#[test]
fn init_excludes_every_channel_seed() {
    let init =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cli/init.rs"))
            .expect("init.rs");
    let start = init.find("const SEEDS").expect("SEEDS");
    let list = &init[start..start + init[start..].find("];").expect("end of SEEDS")];
    let seeds = channel_seeds();
    assert!(
        seeds.contains(&".bosun/intake.toml".to_string()),
        "{seeds:?}"
    );
    for s in &seeds {
        assert!(
            list.contains(&format!("\"{s}\"")),
            "SEEDS in src/cli/init.rs lacks {s}"
        );
    }
}
