//! The verification surface (DRC/LVS/PEX/ERC) and the foundational I/O tier
//! (SPICE reader/writer, GDSII reader/writer, PDK reader).

use philis::io::gds::Gds;
use philis::io::spice::SpiceNetlist;
use philis::{CheckKind, Circuit, Pdk, RunConfig};

fn pdk() -> Pdk {
    Pdk::from_json_str("{\"tech\":\"stub\"}").unwrap()
}

// ---- verification --------------------------------------------------------

#[test]
fn checks_run_on_every_manual_tier() {
    let circuit = Circuit::from_spice_str("m1 d g s b nmos\nm2 d g s b nmos\n").unwrap();
    let constraints = circuit.analyze(&pdk()).unwrap();

    // Tier: constraints (netlist only).
    for kind in [
        CheckKind::Drc,
        CheckKind::Lvs,
        CheckKind::Pex,
        CheckKind::Erc,
    ] {
        let r = constraints.check(kind);
        assert_eq!(r.kind, kind);
        assert!(r.is_clean());
        assert!(r.findings().is_empty());
        assert!(!kind.name().is_empty());
    }

    // Tier: placed (netlist + partial geometry).
    let placed = constraints.place(RunConfig::default()).unwrap();
    assert!(placed.check(CheckKind::Drc).is_clean());
}

#[test]
fn checks_run_after_automation_on_layout() {
    let layout = Circuit::from_spice_str("m1 d g s b nmos\n")
        .unwrap()
        .analyze(&pdk())
        .unwrap()
        .place(RunConfig::default())
        .unwrap()
        .route(RunConfig::default())
        .unwrap();
    assert!(layout.drc().is_clean());
    assert!(layout.lvs().is_clean());
    assert!(layout.pex().is_clean());
    assert!(layout.erc().is_clean());
    assert!(layout.lvs_clean());
    assert!(layout.violations().is_empty());
}

// ---- SPICE reader/writer -------------------------------------------------

#[test]
fn spice_parses_continuations_and_subckts() {
    let text = "* a comment\n.subckt inv a y\nm1 y a 0 0 nmos w=1u\n+ l=180n\n.ends\n";
    let nl = SpiceNetlist::parse(text).unwrap();
    assert!(nl.subckts().contains(&"inv".to_string()));
    // round-trip preserves device count
    let reparsed = SpiceNetlist::parse(&nl.to_spice()).unwrap();
    assert_eq!(reparsed.device_count(), nl.device_count());
}

#[test]
fn spice_from_devices_is_flat() {
    let nl = SpiceNetlist::from_devices(vec!["M1".into(), "M2".into()]);
    assert_eq!(nl.device_count(), 2);
    assert_eq!(nl.top_device_count(), 2);
    assert!(!nl.top_is_empty());
}

// ---- GDSII reader/writer -------------------------------------------------

#[test]
fn gds_round_trips() {
    let mut g = Gds::new("PHILIS");
    assert_eq!(g.lib_name(), "PHILIS");
    g.add_rect("TOP", 1, 0, 0, 1000, 2000);
    g.add_rect("TOP", 2, 500, 500, 100, 100);
    assert!(g.add_cell("SUB"));
    assert!(!g.add_cell("SUB")); // dedup

    let bytes = g.write();
    let back = Gds::read(&bytes).unwrap();
    assert_eq!(back.lib_name(), "PHILIS");
    assert_eq!(back.cell_count(), g.cell_count());
    assert_eq!(back.element_count(), g.element_count());
    assert_eq!(back.element_count(), 2);
}

#[test]
fn gds_rejects_truncated_stream() {
    assert!(Gds::read(&[0x00, 0x06, 0x00, 0x02]).is_err());
}

// ---- PDK reader ----------------------------------------------------------

#[test]
fn pdk_reads_json_and_toml_with_accessors() {
    use philis::io::pdk::PdkFormat;
    let j = Pdk::from_json_str("{\"tech\":\"sky130\",\"layers\":[\"met1\",\"met2\"]}").unwrap();
    assert_eq!(j.tech(), Some("sky130"));
    assert_eq!(j.layers().len(), 2);
    assert_eq!(j.format(), Some(PdkFormat::Json));

    let t = Pdk::from_toml_str("tech = \"gf180\"\nlayers = [\"poly\"]\n").unwrap();
    assert_eq!(t.tech(), Some("gf180"));
    assert_eq!(t.format(), Some(PdkFormat::Toml));
}
