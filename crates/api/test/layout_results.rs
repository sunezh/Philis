//! Layout result inspectors and the Tier-2 engine oracles.

use philis::core::{
    constraint_input, run_constraints, run_placement, run_routing, ConstraintRunInput,
    PlacementRunInput, RoutingRunInput,
};
use philis::{Circuit, Pdk, PlacedInstance, RunConfig, SpiceNetlist, Strategy, Violation};

fn solved() -> philis::Layout {
    Circuit::from_spice_str("m1 d g s b nmos\nm2 d g s b nmos\n")
        .unwrap()
        .analyze(&Pdk::from_json_str("{}").unwrap())
        .unwrap()
        .place(RunConfig::default())
        .unwrap()
        .route(RunConfig::default())
        .unwrap()
}

#[test]
fn layout_inspectors_in_facade_terms() {
    let l = solved();
    // STUB engine: geometric metrics are zeroed but the shape is real.
    assert_eq!(l.width(), 0);
    assert_eq!(l.height(), 0);
    assert_eq!(l.area(), 0);
    assert_eq!(l.wirelength(), 0.0);
    assert!(l.lvs_clean());
    assert_eq!(l.violations(), Vec::<Violation>::new());
    assert_eq!(l.placed_count(), 2);
    assert_eq!(l.routed_segment_count(), 0);
    assert_eq!(l.instances().collect::<Vec<PlacedInstance>>(), vec![]);
}

#[test]
fn layout_flow_is_the_labeled_escape_hatch() {
    let l = solved();
    let flow = l.flow();
    assert_eq!(flow.placement.placed_count, 2);
    assert_eq!(flow.constraints.honored, 2);
}

#[test]
fn tier2_oracles_are_directly_callable() {
    let c = run_constraints(&ConstraintRunInput { device_count: 4 });
    assert_eq!(c.honored, 4);

    let p = run_placement(&PlacementRunInput { device_count: 4 });
    assert_eq!(p.placed_count, 4);
    assert!(p.baseline_used);

    // baseline_used is threaded into routing — here we do it by hand at Tier 2.
    let r = run_routing(&RoutingRunInput {
        net_count: 4,
        baseline_used: p.baseline_used,
    });
    assert_eq!(r.routed_segments, 0);
}

#[test]
fn tier2_namespaces_both_resolve() {
    // flat and `next::` re-exports point at the same oracles.
    let a = philis::constraints::run_constraints(&philis::constraints::ConstraintRunInput {
        device_count: 1,
    });
    let b = philis::next::placer::run_placement(&philis::next::placer::PlacementRunInput {
        device_count: 1,
    });
    let _ = philis::router::run_routing(&philis::router::RoutingRunInput {
        net_count: 1,
        baseline_used: false,
    });
    assert_eq!(a.honored, 1);
    assert_eq!(b.placed_count, 1);
}

#[test]
fn constraint_input_derives_from_netlist() {
    let nl = SpiceNetlist::from_devices(vec!["a".into(), "b".into(), "c".into()]);
    assert_eq!(nl.top_device_count(), 3);
    assert!(!nl.top_is_empty());
    let input = constraint_input(&nl);
    assert_eq!(input.device_count, 3);
}

#[test]
fn placed_instance_and_violation_are_constructible() {
    let v = Violation {
        message: "spacing < min".into(),
    };
    assert!(v.message.contains("spacing"));
    let inst = PlacedInstance {
        name: "M1".into(),
        x: philis::Coord::nm(0),
        y: philis::Coord::nm(0),
    };
    assert_eq!(inst.name, "M1");
    let _ = Strategy::Grid.name();
}
