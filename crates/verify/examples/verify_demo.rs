//! End-to-end demo of the verify crate: runs DRC, LVS connectivity
//! extraction, PEX parasitic extraction, EM, IR drop, and antenna checking
//! on one small two-net layout (a pair of M1 wires connected through a via
//! to M2).
//!
//! Run with:
//!   cargo run -p philis-verify --example verify_demo

use std::collections::HashMap;

use philis_geom::Rect;
use philis_verify::{
    check_antenna, check_em, check_spacing, compute_ir_drop, current_density, extract_connectivity,
    ground_capacitance, via_resistance, wire_resistance, LayerId, NetId, NetParasitics, RuleId,
    SegmentParasitics, ShapeRef,
};

fn main() {
    let m1 = LayerId(1);
    let m2 = LayerId(2);
    let via1 = LayerId(10);

    // Net 1: an M1 wire connected up to an M2 wire through a via.
    let net1_m1 = Rect::new(0, 0, 10_000, 200).unwrap(); // 10um x 0.2um
    let net1_m2 = Rect::new(0, 0, 200, 200).unwrap();
    let via = Rect::new(0, 0, 200, 200).unwrap();

    // Net 2: a parallel M1 wire, 150nm away.
    let net2_m1 = Rect::new(0, 350, 10_000, 550).unwrap();

    println!("--- DRC: spacing between net 1 and net 2 on M1 ---");
    match check_spacing(
        &net1_m1,
        &net2_m1,
        ShapeRef(0),
        ShapeRef(2),
        m1,
        RuleId("M1.S.1".to_string()),
        200,
        false,
    ) {
        Some(v) => println!("Spacing violation: {v:?}"),
        None => println!("Spacing check passed."),
    }

    println!("\n--- LVS: connectivity extraction ---");
    let shapes = vec![
        (ShapeRef(0), m1, net1_m1, Some(NetId(1))),
        (ShapeRef(1), m2, net1_m2, Some(NetId(1))),
        (ShapeRef(2), m1, net2_m1, Some(NetId(2))),
    ];
    let vias = vec![(ShapeRef(3), via1, via)];
    let mut via_connects = HashMap::new();
    via_connects.insert(via1, (m1, m2));

    let (mut tracker, lvs_result) =
        extract_connectivity(shapes.into_iter(), vias.into_iter(), &via_connects);
    println!("net 1 shapes connected: {}", tracker.same_component(0, 1));
    println!("LVS clean: {}", lvs_result.is_clean());

    println!("\n--- PEX: parasitics for net 1 ---");
    let sheet_r = 0.1; // ohms/square
    let area_cap = 0.05; // fF/um^2
    let fringe_cap = 0.05; // fF/um

    let mut net1 = NetParasitics::default();
    net1.add_segment(SegmentParasitics {
        shape: ShapeRef(0),
        layer: m1,
        r: wire_resistance(10_000, 200, sheet_r),
        cg: ground_capacitance(10_000, 200, area_cap, fringe_cap),
        cc: vec![],
        rvia: via_resistance(1, 4.0),
    });
    println!(
        "net 1 totals: r={:.3} ohms, cg={:.3} fF, rvia={:.3} ohms",
        net1.total_r, net1.total_cg, net1.total_rvia
    );

    println!("\n--- EM: current density on net 1's M1 segment ---");
    let current = 4e-3; // 4mA
    let thickness = 100; // 0.1um
    let j = current_density(current, 200, thickness);
    let j_max = 2e-7; // A/nm^2
    match check_em(j, j_max, 10_000, 1_000, ShapeRef(0), m1) {
        Some(v) => println!("EM violation: ratio={:.2}x over limit", v.ratio),
        None => println!("EM check passed."),
    }

    println!("\n--- IR drop: net 1's wire as a resistive mesh ---");
    // VDD(0) -- net1's wire resistance -- node 1, drawing current at node 1.
    let result = compute_ir_drop(&[0, 1], &[(0, 1, net1.total_r)], &[(1, 0.01)], &[0], 1.0);
    println!("converged: {}", result.converged);
    for (node, voltage) in &result.node_voltages {
        println!("node {node}: {voltage:.4} V");
    }
    println!(
        "worst drop: {:.4} V at node {}",
        result.worst_drop, result.worst_node
    );

    println!("\n--- Antenna: net 1's M1 area vs. its connected gate oxide ---");
    let metal_area = net1_m1.area();
    let gate_oxide_area = 5_000; // nm^2
    let max_ratio = 50.0;
    match check_antenna(metal_area, gate_oxide_area, max_ratio, NetId(1), m1) {
        Some(v) => println!(
            "Antenna violation: ratio={:.2} (max {:.2})",
            v.ratio, v.max_ratio
        ),
        None => println!("Antenna check passed."),
    }
}
