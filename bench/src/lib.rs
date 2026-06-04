//! Shared helpers for the Criterion benchmarks in `benches/`.
//!
//! These locate the workspace, build the `philis` executable on demand,
//! enumerate the fixtures in `bench/fixtures/`, generate synthetic inputs for
//! scaling studies, and provide a small **accuracy** harness for comparing the
//! engine's produced layout metrics against expected reference values.
//!
//! We do not have real circuit fixtures yet; when they land, drop them in
//! `bench/fixtures/` (a `name x y` file for the quickstart path, or a `.sp`
//! SPICE file for the API path) and the benches pick them up automatically.

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
        .map(|rd| {
            rd.filter_map(|entry| entry.ok().map(|e| e.path()))
                .filter(|p| p.is_file())
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

/// The standard problem sizes used by the scaling benchmarks.
pub const SCALING_SIZES: &[usize] = &[16, 256, 1024];

/// A deterministic `name x y` quickstart netlist with `n` cells laid out on a
/// pseudo-grid. No RNG — the position is a cheap hash of the index so the
/// bounding box is non-degenerate and stable across runs.
pub fn synthetic_quick_netlist(n: usize) -> String {
    let mut s = String::with_capacity(n * 12);
    for i in 0..n {
        let x = ((i.wrapping_mul(2_654_435_761)) % 1000) as i64 - 500;
        let y = ((i.wrapping_mul(40_503)) % 1000) as i64 - 500;
        s.push_str(&format!("cell{i} {x} {y}\n"));
    }
    s
}

/// A deterministic SPICE-ish netlist with `n` MOSFET devices, for exercising the
/// API flow path. The stub parser only counts device lines, so this is enough to
/// drive the staged flow at a chosen problem size.
pub fn synthetic_spice(n: usize) -> String {
    let mut s = String::with_capacity(n * 20);
    for i in 0..n {
        let kind = if i % 2 == 0 { "nmos" } else { "pmos" };
        s.push_str(&format!("m{i} d{i} g{i} 0 0 {kind}\n"));
    }
    s
}

/// A layout-quality snapshot, used for accuracy comparisons.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub placed: usize,
    pub area: u64,
    pub wirelength: f64,
}

/// The accuracy of a produced result against an expected reference.
#[derive(Debug, Clone, Copy)]
pub struct Accuracy {
    pub placed_match: bool,
    pub area_abs_err: u64,
    pub wirelength_abs_err: f64,
    /// Relative wirelength error in `[0, 1+]` (0.0 when expected is 0 and exact).
    pub wirelength_rel_err: f64,
}

impl Accuracy {
    /// Compare `got` against `expected`.
    pub fn compare(got: Metrics, expected: Metrics) -> Accuracy {
        let wl_abs = (got.wirelength - expected.wirelength).abs();
        let wl_rel = if expected.wirelength.abs() > f64::EPSILON {
            wl_abs / expected.wirelength.abs()
        } else if wl_abs == 0.0 {
            0.0
        } else {
            1.0
        };
        Accuracy {
            placed_match: got.placed == expected.placed,
            area_abs_err: got.area.abs_diff(expected.area),
            wirelength_abs_err: wl_abs,
            wirelength_rel_err: wl_rel,
        }
    }
}
