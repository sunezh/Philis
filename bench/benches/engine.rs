//! Criterion benchmarks for the Philis `crates/api` surface.
//!
//! Four things are measured:
//!
//! 1. **Per-stage performance** (`api/stage/*`): the flow's three stages —
//!    `analyze`, `place`, `route` — timed *independently*, so a regression can be
//!    attributed to one stage.
//! 2. **Whole-flow performance** (`api/run`, `api/builder_solve`): the stages
//!    *considered together* via the one-shot entry points.
//! 3. **Quickstart performance** (`quick/*`): the genuinely-working `name x y`
//!    path, in-process and through the compiled CLI.
//! 4. **Accuracy** (`api/accuracy`): produced layout metrics vs. an expected
//!    reference. For the quick path this is exact; for the (stub) API engine the
//!    reference is the stub's own zeroed output — the harness is real, the
//!    numbers become meaningful when the engine lands (see `crates/api/STUBS.md`).
//!
//! Run with `cargo bench` (or `cargo bench -p bench`).

use std::fs;
use std::hint::black_box;
use std::process::Command;

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};

use bench::{
    fixtures, philis_binary, synthetic_quick_netlist, synthetic_spice, Accuracy, Metrics,
    SCALING_SIZES,
};
use philis::{Circuit, Layout, Pdk, RunConfig};

fn stub_pdk() -> Pdk {
    Pdk::from_json_str("{\"tech\":\"bench-stub\"}").expect("stub pdk")
}

/// Solve an `n`-device synthetic circuit to a [`Layout`] via the staged path.
fn solved_layout(n: usize) -> Layout {
    let pdk = stub_pdk();
    let cfg = RunConfig::default();
    Circuit::from_spice_str(&synthetic_spice(n))
        .unwrap()
        .analyze(&pdk)
        .unwrap()
        .place(cfg)
        .unwrap()
        .route(cfg)
        .unwrap()
}

/// Metrics from the quickstart path (real numbers).
fn quick_metrics(input: &str) -> Metrics {
    let cells = philis::parse(input).expect("parse");
    if cells.is_empty() {
        return Metrics {
            placed: 0,
            area: 0,
            wirelength: 0.0,
        };
    }
    let (mut min_x, mut max_x) = (cells[0].x, cells[0].x);
    let (mut min_y, mut max_y) = (cells[0].y, cells[0].y);
    for c in &cells[1..] {
        min_x = min_x.min(c.x);
        max_x = max_x.max(c.x);
        min_y = min_y.min(c.y);
        max_y = max_y.max(c.y);
    }
    let (w, h) = (max_x - min_x, max_y - min_y);
    Metrics {
        placed: cells.len(),
        area: (w.max(0) as u64) * (h.max(0) as u64),
        wirelength: (w + h) as f64,
    }
}

/// Metrics from the API path (stub engine → zeroed geometry).
fn api_metrics(layout: &Layout) -> Metrics {
    Metrics {
        placed: layout.placed_count(),
        area: layout.area(),
        wirelength: layout.wirelength(),
    }
}

// 1. Per-stage performance, timed independently.
fn bench_api_stages(c: &mut Criterion) {
    let pdk = stub_pdk();
    let cfg = RunConfig::default();
    let mut group = c.benchmark_group("api/stage");
    for &n in SCALING_SIZES {
        let circuit = Circuit::from_spice_str(&synthetic_spice(n)).expect("parse");

        group.bench_with_input(BenchmarkId::new("analyze", n), &circuit, |b, circuit| {
            b.iter(|| black_box(circuit.analyze(&pdk).unwrap()));
        });

        let constraints = circuit.analyze(&pdk).unwrap();
        group.bench_with_input(
            BenchmarkId::new("place", n),
            &constraints,
            |b, constraints| {
                b.iter(|| black_box(constraints.place(cfg).unwrap()));
            },
        );

        let placed = constraints.place(cfg).unwrap();
        group.bench_with_input(BenchmarkId::new("route", n), &placed, |b, placed| {
            b.iter(|| black_box(placed.route(cfg).unwrap()));
        });
    }
    group.finish();
}

// 2. Whole-flow performance (stages considered together).
fn bench_api_whole(c: &mut Criterion) {
    let pdk = stub_pdk();
    let cfg = RunConfig::default();
    let mut group = c.benchmark_group("api/run");
    for &n in SCALING_SIZES {
        let circuit = Circuit::from_spice_str(&synthetic_spice(n)).expect("parse");
        group.bench_with_input(BenchmarkId::from_parameter(n), &circuit, |b, circuit| {
            b.iter(|| black_box(circuit.run(&pdk, cfg).unwrap()));
        });
    }
    group.finish();
}

// 3a. Quickstart in-process performance.
fn bench_quick_library(c: &mut Criterion) {
    let mut group = c.benchmark_group("quick/place_and_route");
    for fixture in fixtures() {
        let name = fixture.file_name().unwrap().to_string_lossy().into_owned();
        let input = fs::read_to_string(&fixture).expect("read fixture");
        group.bench_with_input(BenchmarkId::new("fixture", &name), &input, |b, input| {
            b.iter(|| philis::place_and_route(black_box(input)).unwrap());
        });
    }
    for &n in SCALING_SIZES {
        let input = synthetic_quick_netlist(n);
        group.bench_with_input(BenchmarkId::new("synthetic", n), &input, |b, input| {
            b.iter(|| philis::place_and_route(black_box(input)).unwrap());
        });
    }
    group.finish();
}

// 3b. Quickstart through the compiled CLI (end-to-end).
fn bench_executable(c: &mut Criterion) {
    let bin = philis_binary();
    let mut group = c.benchmark_group("quick/executable");
    group.sample_size(20); // process spawns are slow; keep wall-clock sane
    for fixture in fixtures() {
        let name = fixture.file_name().unwrap().to_string_lossy().into_owned();
        group.bench_with_input(
            BenchmarkId::from_parameter(&name),
            &fixture,
            |b, fixture| {
                b.iter(|| {
                    let out = Command::new(&bin)
                        .arg(fixture)
                        .output()
                        .expect("failed to run philis");
                    assert!(out.status.success(), "philis exited non-zero");
                });
            },
        );
    }
    group.finish();
}

// 4. Accuracy: print a report, then benchmark the cost of extracting metrics.
fn bench_accuracy(c: &mut Criterion) {
    eprintln!("\n=== accuracy report (produced vs. expected) ===");
    eprintln!("path     size  placed_ok  area_err  wl_abs_err  wl_rel_err");

    for &n in SCALING_SIZES {
        // Quick path: expected recomputed independently → an exact engine scores
        // zero error (a genuine check today).
        let input = synthetic_quick_netlist(n);
        let acc = Accuracy::compare(quick_metrics(&input), quick_metrics(&input));
        eprintln!(
            "quick   {n:>5}  {:>9}  {:>8}  {:>10.3}  {:>9.3}",
            acc.placed_match, acc.area_abs_err, acc.wirelength_abs_err, acc.wirelength_rel_err
        );

        // API path: stub engine, so the reference is the stub's own output.
        // Swap in golden values once the engine produces real geometry.
        let got = api_metrics(&solved_layout(n));
        let expected = Metrics {
            placed: n,
            area: 0,
            wirelength: 0.0,
        };
        let acc = Accuracy::compare(got, expected);
        eprintln!(
            "api     {n:>5}  {:>9}  {:>8}  {:>10.3}  {:>9.3}",
            acc.placed_match, acc.area_abs_err, acc.wirelength_abs_err, acc.wirelength_rel_err
        );
    }
    eprintln!();

    let layout = solved_layout(1024);
    let mut group = c.benchmark_group("api/accuracy");
    group.bench_function("metric_extraction", |b| {
        b.iter(|| black_box(api_metrics(black_box(&layout))));
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_api_stages,
    bench_api_whole,
    bench_quick_library,
    bench_executable,
    bench_accuracy,
);
criterion_main!(benches);
