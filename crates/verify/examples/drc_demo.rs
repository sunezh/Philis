//! Small demo of the DRC predicate library: constructs a couple of shapes
//! and runs a few checks against them, printing any violations found.
//!
//! Run with:
//!   cargo run -p philis-verify --example drc_demo

use philis_geom::Rect;
use philis_verify::{check_overlap, check_spacing, check_width, LayerId, RuleId, ShapeRef};

fn main() {
    let metal1 = LayerId(1);

    // Two M1 wires, 80nm apart, on a layer with a 100nm minimum spacing rule.
    let wire_a = Rect::new(0, 0, 200, 100).unwrap();
    let wire_b = Rect::new(280, 0, 480, 100).unwrap();

    match check_spacing(
        &wire_a,
        &wire_b,
        ShapeRef(0),
        ShapeRef(1),
        metal1,
        RuleId("M1.S.1".to_string()),
        100,
        false,
    ) {
        Some(v) => println!("Spacing violation: {v:?}"),
        None => println!("Spacing check passed."),
    }

    // A narrow M1 shape: 60nm wide, but the minimum width rule is 80nm.
    let narrow = Rect::new(0, 0, 60, 500).unwrap();
    match check_width(
        &narrow,
        ShapeRef(2),
        metal1,
        RuleId("M1.W.1".to_string()),
        80,
    ) {
        Some(v) => println!("Width violation: {v:?}"),
        None => println!("Width check passed."),
    }

    // Two overlapping shapes on different nets.
    let shape_c = Rect::new(0, 0, 100, 100).unwrap();
    let shape_d = Rect::new(50, 50, 150, 150).unwrap();
    match check_overlap(
        &shape_c,
        &shape_d,
        ShapeRef(3),
        ShapeRef(4),
        metal1,
        RuleId("M1.O.1".to_string()),
        false,
    ) {
        Some(v) => println!("Overlap violation: {v:?}"),
        None => println!("Overlap check passed."),
    }
}
