//! End-to-end integration across the tiers. Run with `cargo test -p api`.
//!
//! This file walks the integration-discontinuity ladder from the design docs:
//! each step is a strict superset of the previous one, and stopping at any rung
//! is legal.

use philis::{
    parse, place_and_route, Circuit, CircuitDef, Constraint, Length, MosfetPort, Objective, Pdk,
    RunConfig,
};

fn pdk() -> Pdk {
    Pdk::from_json_str("{\"tech\":\"stub\"}").unwrap()
}

#[test]
fn step1_quickstart_path() {
    // The genuinely-working Tier 0 quickstart.
    let netlist = "inv0 0 0\ninv1 10 0\ninv2 10 5\n";
    assert_eq!(
        place_and_route(netlist).unwrap(),
        "placed 3 cells, bbox 10x5, hpwl 15"
    );
    assert_eq!(parse("# c\na 1 2\nb 3 4\n").unwrap().len(), 2);
}

#[test]
fn step2_spice_one_shot() {
    let circuit = Circuit::from_spice_str("m1 d g s b nmos\nm2 d g s b nmos\n").unwrap();
    let flow = circuit.run(&pdk(), RunConfig::default()).unwrap();
    assert_eq!(flow.placement.placed_count, 2);
}

#[test]
fn step3_staged_chain_matches_one_shot() {
    // Tier 1: analyze -> place -> route, reachable in the façade's own words.
    let circuit = Circuit::from_spice_str("m1 d g s b nmos\nm2 d g s b nmos\n").unwrap();
    let staged = circuit
        .analyze(&pdk())
        .unwrap()
        .place(RunConfig::default())
        .unwrap()
        .route(RunConfig::default())
        .unwrap();
    let one_shot = circuit.run(&pdk(), RunConfig::default()).unwrap();
    assert_eq!(staged.placed_count(), one_shot.placement.placed_count);
}

#[test]
fn step4_builder_to_layout() {
    // Tier 0 builder path, then additive result inspection.
    let layout = CircuitDef::new()
        .nmos("M1", |d| d.w(Length::um(2.0)).l(Length::nm(180.0)).nf(2))
        .nmos("M2", |d| d.w(Length::um(2.0)).l(Length::nm(180.0)).nf(2))
        .net("GATE", |n| {
            n.connect("M1", MosfetPort::Gate)
                .connect("M2", MosfetPort::Gate)
        })
        .constrain(Constraint::Symmetric("M1".into(), "M2".into()))
        .build()
        .unwrap()
        .solve(&pdk(), Objective::Area)
        .unwrap();

    let _ = layout.area();
    let _ = layout.wirelength();
    let _ = layout.violations();
    assert!(layout.lvs_clean());
}
