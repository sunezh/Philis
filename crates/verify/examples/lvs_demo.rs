//! Small demo of LVS connectivity extraction: builds a few layout shapes,
//! extracts connectivity, and prints the resulting opens/shorts.
//!
//! Run with:
//!   cargo run -p philis-verify --example lvs_demo

use std::collections::HashMap;

use philis_geom::Rect;
use philis_verify::{extract_connectivity, LayerId, NetId, ShapeRef};

fn main() {
    let m1 = LayerId(1);
    let m2 = LayerId(2);
    let via1 = LayerId(10);

    println!("--- Case 1: clean net, connected via a via ---");
    let shapes = vec![
        (
            ShapeRef(0),
            m1,
            Rect::new(0, 0, 100, 100).unwrap(),
            Some(NetId(1)),
        ),
        (
            ShapeRef(1),
            m2,
            Rect::new(0, 0, 100, 100).unwrap(),
            Some(NetId(1)),
        ),
    ];
    let vias = vec![(ShapeRef(2), via1, Rect::new(40, 40, 60, 60).unwrap())];
    let mut via_connects = HashMap::new();
    via_connects.insert(via1, (m1, m2));

    let (mut tracker, result) =
        extract_connectivity(shapes.into_iter(), vias.into_iter(), &via_connects);
    println!("same component: {}", tracker.same_component(0, 1));
    println!("clean: {}", result.is_clean());

    println!("\n--- Case 2: open net (same net, not connected) ---");
    let shapes = vec![
        (
            ShapeRef(0),
            m1,
            Rect::new(0, 0, 100, 100).unwrap(),
            Some(NetId(1)),
        ),
        (
            ShapeRef(1),
            m1,
            Rect::new(200, 0, 300, 100).unwrap(),
            Some(NetId(1)),
        ),
    ];
    let (_tracker, result) =
        extract_connectivity(shapes.into_iter(), std::iter::empty(), &HashMap::new());
    println!("opens: {:?}", result.opens);

    println!("\n--- Case 3: short (different nets, touching shapes) ---");
    let shapes = vec![
        (
            ShapeRef(0),
            m1,
            Rect::new(0, 0, 100, 100).unwrap(),
            Some(NetId(1)),
        ),
        (
            ShapeRef(1),
            m1,
            Rect::new(100, 0, 200, 100).unwrap(),
            Some(NetId(2)),
        ),
    ];
    let (_tracker, result) =
        extract_connectivity(shapes.into_iter(), std::iter::empty(), &HashMap::new());
    println!("shorts: {:?}", result.shorts);
}
