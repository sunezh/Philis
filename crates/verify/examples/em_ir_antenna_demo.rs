//! Small demo of EM/IR/antenna checking: checks a wire's current density
//! against its EM limit, solves a small resistive mesh for IR drop, and
//! checks a net's antenna ratio.
//!
//! Run with:
//!   cargo run -p philis-verify --example em_ir_antenna_demo

use philis_verify::{
    check_antenna, check_em, compute_ir_drop, current_density, LayerId, NetId, ShapeRef,
};

fn main() {
    let m1 = LayerId(1);

    println!("--- EM check: wire current density vs. J_max ---");
    let current = 5e-3; // 5mA
    let width = 200; // 0.2um
    let thickness = 100; // 0.1um
    let j = current_density(current, width, thickness);
    let j_max = 2e-7; // A/nm^2
    let segment_length = 10_000; // 10um
    let blech_length = 1_000; // 1um

    println!("current density: {j:.3e} A/nm^2 (limit {j_max:.3e})");
    match check_em(j, j_max, segment_length, blech_length, ShapeRef(0), m1) {
        Some(v) => println!("EM violation: ratio={:.2}x over limit", v.ratio),
        None => println!("EM check passed."),
    }

    println!("\n--- IR drop: a small resistive mesh ---");
    // VDD(0) --10ohm-- node 1 --10ohm-- node 2, with current sinks along the way.
    let result = compute_ir_drop(
        &[0, 1, 2],
        &[(0, 1, 10.0), (1, 2, 10.0)],
        &[(1, 0.01), (2, 0.01)],
        &[0],
        1.0,
    );
    println!("converged: {}", result.converged);
    for (node, voltage) in &result.node_voltages {
        println!("node {node}: {voltage:.4} V");
    }
    println!(
        "worst drop: {:.4} V at node {}",
        result.worst_drop, result.worst_node
    );

    println!("\n--- Antenna check: metal area vs. gate oxide area ---");
    let metal_area = 500_000; // nm^2
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
