//! Unit value types and the port vocabulary.

use philis::{
    port_ref_from_name, valid_port_names, BjtPort, Coord, DeviceKind, Distance, Length, MosfetPort,
    PassivePort, PortRef,
};

#[test]
fn length_constructors_and_conversions() {
    assert_eq!(Length::m(1.0).as_meters(), 1.0);
    assert_eq!(Length::um(2.0).to_nm_i64(), 2_000);
    assert_eq!(Length::nm(180.0).to_nm_i64(), 180);
    assert_eq!(Length::pm(1000.0).to_nm_i64(), 1);
}

#[test]
fn distance_and_coord_vocabulary_is_unified() {
    assert_eq!(Distance::new(5).as_nm(), 5);
    assert_eq!(Distance::nm(5).as_nm(), 5);
    assert_eq!(Distance::um(3).as_nm(), 3_000);
    assert_eq!(Coord::new(5).as_nm(), 5);
    assert_eq!(Coord::nm(5).as_nm(), 5);
    assert_eq!(Coord::um(3).as_nm(), 3_000);
    assert_eq!(Distance::default(), Distance(0));
    assert_eq!(Coord::default(), Coord(0));
}

#[test]
fn length_converts_into_distance_and_coord() {
    let d: Distance = Length::um(1.0).into();
    assert_eq!(d.as_nm(), 1_000);
    let c: Coord = Length::nm(250.0).into();
    assert_eq!(c.as_nm(), 250);
}

#[test]
fn mosfet_port_idx_name_and_ref() {
    assert_eq!(MosfetPort::Gate.idx(), 0);
    assert_eq!(MosfetPort::Drain.idx(), 1);
    assert_eq!(MosfetPort::Source.idx(), 2);
    assert_eq!(MosfetPort::Bulk.idx(), 3);
    assert_eq!(MosfetPort::Gate.name(), "gate");
    let r: PortRef = MosfetPort::Bulk.into();
    assert_eq!(r.idx(), 3);
    assert_eq!(r.name(), "bulk");
}

#[test]
fn passive_and_bjt_ports() {
    assert_eq!(PassivePort::Plus.idx(), 0);
    assert_eq!(PassivePort::Minus.idx(), 1);
    assert_eq!(PassivePort::Plus.name(), "plus");
    assert_eq!(BjtPort::Collector.idx(), 0);
    assert_eq!(BjtPort::Base.idx(), 1);
    assert_eq!(BjtPort::Emitter.idx(), 2);
    assert_eq!(BjtPort::Emitter.name(), "emitter");
    let _: PortRef = PassivePort::Minus.into();
    let _: PortRef = BjtPort::Base.into();
}

#[test]
fn port_ref_from_name_handles_aliases() {
    // "body" is an alias for bulk; lookups are case-insensitive.
    assert_eq!(
        port_ref_from_name(DeviceKind::Nmos, "body").unwrap().name(),
        "bulk"
    );
    assert_eq!(
        port_ref_from_name(DeviceKind::Pmos, "GATE").unwrap().name(),
        "gate"
    );
    assert_eq!(
        port_ref_from_name(DeviceKind::Resistor, "+").unwrap().name(),
        "plus"
    );
    assert_eq!(
        port_ref_from_name(DeviceKind::Bjt, "c").unwrap().name(),
        "collector"
    );
    assert!(port_ref_from_name(DeviceKind::Nmos, "nonsense").is_none());
}

#[test]
fn valid_port_names_per_kind() {
    assert_eq!(
        valid_port_names(DeviceKind::Nmos),
        &["gate", "drain", "source", "bulk"]
    );
    assert_eq!(valid_port_names(DeviceKind::Capacitor), &["plus", "minus"]);
    assert_eq!(
        valid_port_names(DeviceKind::Bjt),
        &["collector", "base", "emitter"]
    );
    // every advertised name resolves
    for kind in [DeviceKind::Nmos, DeviceKind::Resistor, DeviceKind::Bjt] {
        for name in valid_port_names(kind) {
            assert!(port_ref_from_name(kind, name).is_some());
        }
    }
}
