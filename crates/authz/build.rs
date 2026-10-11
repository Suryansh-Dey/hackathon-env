//! Embeds every `policies/*.cedar` file (one policy per file; the file stem is
//! the policy ID) so the crate and the CDK deployer read the same sources.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let dir = manifest.join("../../policies").canonicalize().expect("policies/ directory exists");
    println!("cargo:rerun-if-changed={}", dir.display());

    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("read policies/")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "cedar"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .cedar policies found in {}", dir.display());

    let mut out =
        String::from("/// `(policy id, Cedar source)` for every file in `policies/`.\npub const POLICIES: &[(&str, &str)] = &[\n");
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        let id = f.file_stem().and_then(|s| s.to_str()).expect("utf-8 file name");
        assert!(
            id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "policy file name {id:?} must be kebab-case"
        );
        out.push_str(&format!("    ({id:?}, include_str!({:?})),\n", f.display().to_string()));
    }
    out.push_str("];\n");
    let dest = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("policies.rs");
    fs::write(dest, out).expect("write policies.rs");
}
