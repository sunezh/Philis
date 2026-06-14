//! Small demo of PEX parasitic extraction: computes resistance/capacitance
//! for a couple of wire segments, aggregates them per-net, and runs outlier
//! detection.
//!
//! Run with:
//!   cargo run -p philis-verify --example pex_demo

use std::collections::HashMap;

use philis_verify::{
    detect_outliers, ground_capacitance, same_layer_coupling, via_resistance, wire_resistance,
    LayerId, MatchedBudget, NetId, NetParasitics, SegmentParasitics, ShapeRef,
};

fn main() {
    let m1 = LayerId(1);

    println!("--- Net A: a single long, narrow wire ---");
    let length = 50_000; // 50um
    let width = 200; // 0.2um
    let sheet_r = 0.1; // ohms/square
    let area_cap = 0.05; // fF/um^2
    let fringe_cap = 0.05; // fF/um

    let r = wire_resistance(length, width, sheet_r);
    let cg = ground_capacitance(length, width, area_cap, fringe_cap);
    let cc = same_layer_coupling(length, 200, |spacing| 1.0 / spacing as f64 * 50.0);

    println!("resistance:  {r:.3} ohms");
    println!("ground cap:  {cg:.3} fF");
    println!("coupling cap to net B: {cc:.3} fF");

    let mut net_a = NetParasitics::default();
    net_a.add_segment(SegmentParasitics {
        shape: ShapeRef(0),
        layer: m1,
        r,
        cg,
        cc: vec![(NetId(2), cc)],
        rvia: via_resistance(2, 4.0),
    });

    println!(
        "Net A totals: r={:.3} ohms, cg={:.3} fF, cc={:.3} fF, rvia={:.3} ohms",
        net_a.total_r, net_a.total_cg, net_a.total_cc, net_a.total_rvia
    );

    println!("\n--- Net B: a short, wide wire (matched pair with A) ---");
    let mut net_b = NetParasitics::default();
    net_b.add_segment(SegmentParasitics {
        shape: ShapeRef(1),
        layer: m1,
        r: wire_resistance(5_000, 400, sheet_r),
        cg: ground_capacitance(5_000, 400, area_cap, fringe_cap),
        cc: vec![(NetId(1), cc)],
        rvia: via_resistance(2, 4.0),
    });
    println!(
        "Net B totals: r={:.3} ohms, cg={:.3} fF, cc={:.3} fF, rvia={:.3} ohms",
        net_b.total_r, net_b.total_cg, net_b.total_cc, net_b.total_rvia
    );

    println!("\n--- Outlier detection ---");
    let mut nets = HashMap::new();
    nets.insert(NetId(1), net_a);
    nets.insert(NetId(2), net_b);

    let r_threshold = 20.0; // ohms
    let cc_threshold = 10.0; // fF
    let matched_groups = vec![(
        vec![NetId(1), NetId(2)],
        MatchedBudget {
            max_r_delta: 5.0,
            max_cg_delta: f64::INFINITY,
            max_cc_delta: f64::INFINITY,
        },
    )];

    let outliers = detect_outliers(&nets, r_threshold, cc_threshold, &matched_groups);
    for outlier in &outliers {
        println!(
            "{:?}: net {:?} actual={:.3}{} threshold={:.3}{}",
            outlier.kind,
            outlier.net,
            outlier.actual,
            outlier.unit,
            outlier.threshold,
            outlier.unit
        );
    }
    if outliers.is_empty() {
        println!("no outliers");
    }
}
