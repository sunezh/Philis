//! The façade-owned `Axis`, `parse_axis`, and the `Constraint` taxonomy.

use philis::{parse_axis, Axis, Constraint, Coord, Distance};

#[test]
fn axis_is_facade_owned() {
    // Building an ordering constraint needs no engine import — only `philis::Axis`.
    let c = Constraint::Order("A".into(), "B".into(), Axis::X);
    assert_eq!(c.kind(), "Order");
    assert_eq!(Axis::X.name(), "x");
    assert_eq!(Axis::Y.name(), "y");
}

#[test]
fn parse_axis_accepts_synonyms() {
    assert_eq!(parse_axis("x").unwrap(), Axis::X);
    assert_eq!(parse_axis("horizontal").unwrap(), Axis::X);
    assert_eq!(parse_axis("Y").unwrap(), Axis::Y);
    assert_eq!(parse_axis("VERTICAL").unwrap(), Axis::Y);
    assert!(parse_axis("diagonal").is_err());
}

#[test]
fn supported_constraints_are_classified() {
    let supported = [
        Constraint::Symmetric("a".into(), "b".into()),
        Constraint::Matching("a".into(), "b".into()),
        Constraint::SelfSymmetric("a".into()),
        Constraint::SymmetricGroup(vec![("a".into(), "b".into())]),
        Constraint::Order("a".into(), "b".into(), Axis::X),
        Constraint::Align(vec!["a".into(), "b".into()], Axis::Y),
        Constraint::DistanceConstraint("a".into(), "b".into(), Distance::nm(100)),
        Constraint::GuardRing(vec!["a".into()], "ring".into()),
        Constraint::NetShield("net".into()),
        Constraint::SetNetClass("net".into(), "power".into()),
    ];
    assert_eq!(supported.len(), 10);
    for c in &supported {
        assert!(c.is_engine_supported(), "{} should be supported", c.kind());
    }
}

#[test]
fn unsupported_constraints_are_classified() {
    let unsupported = [
        Constraint::SignalFlow(vec!["a".into()]),
        Constraint::AspectRatio(1.0, 2.0),
        Constraint::CellBoundary(Distance::nm(1), Distance::nm(2)),
        Constraint::FixPosition("a".into(), Coord::nm(0), Coord::nm(0)),
        Constraint::PlaceOnGrid("a".into(), Distance::nm(10)),
        Constraint::PlaceOnBoundary("a".into(), "north".into()),
        Constraint::Group(vec!["a".into()], "g".into()),
        Constraint::PortLocation("a".into(), "net".into(), 0.5),
        Constraint::NetMatch("a".into(), "b".into(), Distance::nm(5)),
        Constraint::MultiWire("net".into(), 3),
        Constraint::DoNotRoute("net".into()),
        Constraint::ChargeFlow("net".into()),
    ];
    assert_eq!(unsupported.len(), 12);
    for c in &unsupported {
        assert!(
            !c.is_engine_supported(),
            "{} should be unsupported",
            c.kind()
        );
    }
}

#[test]
fn kind_tags_are_unique_across_all_22_variants() {
    let mut kinds = vec![
        Constraint::Symmetric("a".into(), "b".into()).kind(),
        Constraint::Matching("a".into(), "b".into()).kind(),
        Constraint::SelfSymmetric("a".into()).kind(),
        Constraint::SymmetricGroup(vec![]).kind(),
        Constraint::Order("a".into(), "b".into(), Axis::X).kind(),
        Constraint::Align(vec![], Axis::X).kind(),
        Constraint::DistanceConstraint("a".into(), "b".into(), Distance::nm(1)).kind(),
        Constraint::GuardRing(vec![], "r".into()).kind(),
        Constraint::NetShield("n".into()).kind(),
        Constraint::SetNetClass("n".into(), "c".into()).kind(),
        Constraint::SignalFlow(vec![]).kind(),
        Constraint::AspectRatio(1.0, 1.0).kind(),
        Constraint::CellBoundary(Distance::nm(1), Distance::nm(1)).kind(),
        Constraint::FixPosition("a".into(), Coord::nm(0), Coord::nm(0)).kind(),
        Constraint::PlaceOnGrid("a".into(), Distance::nm(1)).kind(),
        Constraint::PlaceOnBoundary("a".into(), "n".into()).kind(),
        Constraint::Group(vec![], "g".into()).kind(),
        Constraint::PortLocation("a".into(), "n".into(), 0.0).kind(),
        Constraint::NetMatch("a".into(), "b".into(), Distance::nm(1)).kind(),
        Constraint::MultiWire("n".into(), 1).kind(),
        Constraint::DoNotRoute("n".into()).kind(),
        Constraint::ChargeFlow("n".into()).kind(),
    ];
    assert_eq!(kinds.len(), 22);
    kinds.sort_unstable();
    kinds.dedup();
    assert_eq!(kinds.len(), 22, "kind() tags must be unique");
}
