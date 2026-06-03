//! Shared helpers for the Criterion benchmarks in `benches/`.
//!
//! These locate the workspace, build the `philis` executable on demand, and
//! enumerate the fixtures in `bench/fixtures/`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The workspace root (the parent of this `bench/` crate).
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("bench/ has a parent")
        .to_path_buf()
}

/// Build the `philis` binary in release mode (if needed) and return its path.
///
/// This is what makes `bench/` benchmark *the executable* rather than just the
/// linked library — we shell out to the real, compiled CLI.
pub fn philis_binary() -> PathBuf {
    let root = workspace_root();
    let cargo = option_env!("CARGO").unwrap_or("cargo");
    let status = Command::new(cargo)
        .args(["build", "--release", "-p", "api", "--bin", "philis"])
        .current_dir(&root)
        .status()
        .expect("failed to spawn cargo to build the philis binary");
    assert!(status.success(), "cargo failed to build the philis binary");

    let bin = root.join("target").join("release").join("philis");
    assert!(bin.exists(), "philis binary not found at {}", bin.display());
    bin
}

/// All fixture files under `bench/fixtures/`, sorted by name.
pub fn fixtures() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    files
}
