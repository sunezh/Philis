//! Criterion benchmarks for the Philis engine.
//!
//! Two groups, covering two of the crate's faces:
//!   * `library/place_and_route` — the engine in-process, via the linked rlib.
//!   * `executable/philis`       — the compiled CLI end-to-end, via subprocess.
//!
//! Run with `cargo bench` (or `cargo bench -p bench`).

use std::fs;
use std::hint::black_box;
use std::process::Command;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

use bench::{fixtures, philis_binary};

fn bench_library(c: &mut Criterion) {
    let mut group = c.benchmark_group("library/place_and_route");
    for fixture in fixtures() {
        let name = fixture.file_name().unwrap().to_string_lossy().into_owned();
        let input = fs::read_to_string(&fixture).expect("read fixture");
        group.bench_with_input(BenchmarkId::from_parameter(&name), &input, |b, input| {
            b.iter(|| philis::place_and_route(black_box(input)).unwrap());
        });
    }
    group.finish();
}

fn bench_executable(c: &mut Criterion) {
    let bin = philis_binary();
    let mut group = c.benchmark_group("executable/philis");
    // Process spawns are orders of magnitude slower than the in-process call,
    // so keep the sample size modest to keep wall-clock reasonable.
    group.sample_size(20);
    for fixture in fixtures() {
        let name = fixture.file_name().unwrap().to_string_lossy().into_owned();
        group.bench_with_input(BenchmarkId::from_parameter(&name), &fixture, |b, fixture| {
            b.iter(|| {
                let out = Command::new(&bin)
                    .arg(fixture)
                    .output()
                    .expect("failed to run philis");
                assert!(out.status.success(), "philis exited non-zero");
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_library, bench_executable);
criterion_main!(benches);
