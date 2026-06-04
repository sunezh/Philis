//! The programmatic builder path: every device kind, nets, validation errors,
//! warnings, and both solve entry points.

use philis::{
    BuildError, BuildResult, BuildWarning, CircuitDef, Constraint, HintBuilder, Length, MosfetPort,
    Objective, PassivePort, Pdk, Strategy,
};

fn pdk() -> Pdk {
    Pdk::from_json_str("{}").unwrap()
}

#[test]
fn every_device_kind_builds() {
    let built = CircuitDef::new()
        .nmos("M1", |d| d.w(Length::um(1.0)).l(Length::nm(180.0)).nf(1))
        .pmos("M2", |d| d.w(Length::um(2.0)))
        .resistor("R1", |d| d.r(1000.0))
        .capacitor("C1", |d| d.c(1e-15))
        .bjt("Q1", |d| d)
        .diode("D1", |d| d.area(Length::um(0.5)).pj(Length::um(0.1)))
        .generic("X1", |d| d)
        .build()
        .unwrap();
    assert_eq!(built.device_count(), 7);
    assert_eq!(built.net_count(), 0);
    assert!(built.netlist().device_count() == 7);
}

#[test]
fn device_builder_reports_kind() {
    use philis::DeviceKind;
    CircuitDef::new().nmos("M1", |d| {
        assert_eq!(d.device_kind(), Some(DeviceKind::Nmos));
        d
    });
}

#[test]
fn nets_and_connection_counting() {
    let built = CircuitDef::new()
        .nmos("M1", |d| d)
        .nmos("M2", |d| d)
        .net("GATE", |n| {
            assert_eq!(n.connection_count(), 0);
            n.connect("M1", MosfetPort::Gate).connect("M2", MosfetPort::Gate)
        })
        .build()
        .unwrap();
    assert_eq!(built.net_count(), 1);
}

#[test]
fn duplicate_device_is_error() {
    let err = CircuitDef::new()
        .nmos("M1", |d| d)
        .nmos("M1", |d| d)
        .build()
        .unwrap_err();
    assert!(err.contains(|e| matches!(e, BuildError::DuplicateDevice(n) if n == "M1")));
    assert_eq!(err.errors().len(), 1);
    assert!(!format!("{err}").is_empty());
}

#[test]
fn duplicate_net_is_error() {
    let err = CircuitDef::new()
        .nmos("M1", |d| d)
        .net("N", |n| n.connect("M1", MosfetPort::Gate))
        .net("N", |n| n.connect("M1", MosfetPort::Drain))
        .build()
        .unwrap_err();
    assert!(err.contains(|e| matches!(e, BuildError::DuplicateNet(_))));
}

#[test]
fn connect_to_unknown_device_is_error() {
    let err = CircuitDef::new()
        .nmos("M1", |d| d)
        .net("N", |n| n.connect("GHOST", MosfetPort::Gate))
        .build()
        .unwrap_err();
    assert!(err.contains(|e| matches!(e, BuildError::UnknownDevice(n) if n == "GHOST")));
}

#[test]
fn terminal_conflict_is_error() {
    // M1.Gate bound to two different nets.
    let err = CircuitDef::new()
        .nmos("M1", |d| d)
        .nmos("M2", |d| d)
        .net("A", |n| n.connect("M1", MosfetPort::Gate).connect("M2", MosfetPort::Gate))
        .net("B", |n| n.connect("M1", MosfetPort::Gate).connect("M2", MosfetPort::Drain))
        .build()
        .unwrap_err();
    assert!(err.contains(|e| matches!(e, BuildError::TerminalConflict { device, .. } if device == "M1")));
}

#[test]
fn floating_net_is_a_warning_not_an_error() {
    let built = CircuitDef::new()
        .nmos("M1", |d| d)
        .net("DANGLE", |n| n.connect("M1", MosfetPort::Gate)) // only one terminal
        .build()
        .unwrap();
    assert!(built
        .warnings()
        .iter()
        .any(|w| matches!(w, BuildWarning::FloatingNet(n) if n == "DANGLE")));
}

#[test]
fn unsupported_constraint_warns_instead_of_silently_dropping() {
    // This is the design's headline transparency fix.
    let built = CircuitDef::new()
        .nmos("M1", |d| d)
        .nmos("M2", |d| d)
        .net("G", |n| n.connect("M1", MosfetPort::Gate).connect("M2", MosfetPort::Gate))
        .constrain(Constraint::Symmetric("M1".into(), "M2".into())) // supported -> no warning
        .constrain(Constraint::DoNotRoute("VDD".into())) // unsupported -> warning
        .build()
        .unwrap();
    let unsupported: Vec<_> = built
        .warnings()
        .iter()
        .filter_map(|w| match w {
            BuildWarning::UnsupportedConstraint(k) => Some(*k),
            _ => None,
        })
        .collect();
    assert_eq!(unsupported, vec!["DoNotRoute"]);
    assert_eq!(built.constraints().len(), 2); // both retained on the value
}

#[test]
fn constraint_referencing_unknown_device_is_error() {
    let err = CircuitDef::new()
        .nmos("M1", |d| d)
        .constrain(Constraint::Symmetric("M1".into(), "GHOST".into()))
        .build()
        .unwrap_err();
    assert!(err.contains(|e| matches!(e, BuildError::UnknownDevice(n) if n == "GHOST")));
}

#[test]
fn passive_ports_connect() {
    CircuitDef::new()
        .resistor("R1", |d| d.r(1.0))
        .net("P", |n| n.connect("R1", PassivePort::Plus))
        .net("M", |n| n.connect("R1", PassivePort::Minus))
        .build()
        .unwrap();
}

#[test]
fn solve_and_solve_with_hints() {
    let built = CircuitDef::new()
        .nmos("M1", |d| d)
        .nmos("M2", |d| d)
        .build()
        .unwrap();

    let l1 = built.solve(&pdk(), Objective::Area).unwrap();
    assert_eq!(l1.placed_count(), 2);

    let l2 = built
        .solve_with(&pdk(), Objective::Speed, |h: &mut HintBuilder| {
            h.seed(7)
                .fix_position("M1", Length::um(0.0), Length::um(0.0))
                .arrange(&["M1", "M2"], Strategy::CommonCentroid);
        })
        .unwrap();
    assert_eq!(l2.placed_count(), 2);
}

#[test]
fn build_result_is_constructible() {
    // `BuildResult` is a reserved value-carrying alternative to reading warnings.
    let circuit = CircuitDef::new().nmos("M1", |d| d).build().unwrap();
    let warnings = circuit.warnings().to_vec();
    let r = BuildResult { circuit, warnings };
    assert_eq!(r.circuit.device_count(), 1);
    assert_eq!(r.warnings.len(), r.circuit.warnings().len());
}
