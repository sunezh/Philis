//! Smoke tests for the SPICE-driven flow path: PDK loading, circuit parsing,
//! subckt selection, the one-shot run, the staged tiers, and error paths.

use std::io::Write;

use philis::{ApiError, Circuit, Pdk, RunConfig};

fn temp_file(name: &str, contents: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("philis_test_{}_{}", std::process::id(), name));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(contents.as_bytes()).unwrap();
    path
}

fn pdk() -> Pdk {
    Pdk::from_json_str("{\"tech\":\"stub\"}").unwrap()
}

#[test]
fn pdk_from_str_parses_schema_and_warns() {
    // A complete schema parses cleanly and exposes its fields.
    let p = Pdk::from_json_str("{\"tech\":\"sky130\",\"db_unit_nm\":1,\"layers\":[\"met1\"]}")
        .unwrap();
    assert!(p.warnings().is_empty());
    assert_eq!(p.tech(), Some("sky130"));
    assert_eq!(p.db_unit_nm(), Some(1));
    assert_eq!(p.layers(), &["met1".to_string()]);

    // A schema missing `tech`, or text that isn't JSON, warns rather than failing.
    assert!(!Pdk::from_json_str("{}").unwrap().warnings().is_empty());
    assert!(!Pdk::from_json_str("not json").unwrap().warnings().is_empty());
}

#[test]
fn pdk_from_toml_str() {
    let p = Pdk::from_toml_str("tech = \"gf180\"\ndb_unit_nm = 5\n").unwrap();
    assert_eq!(p.tech(), Some("gf180"));
    assert_eq!(p.db_unit_nm(), Some(5));
}

#[test]
fn pdk_empty_is_error() {
    assert!(matches!(Pdk::from_json_str("   "), Err(ApiError::Pdk(_))));
}

#[test]
fn pdk_from_file_path_and_env() {
    let path = temp_file("pdk.json", "{\"tech\":\"x\"}");
    assert!(Pdk::from_json_file(&path).is_ok());
    assert!(Pdk::from_path(&path).is_ok()); // alias

    std::env::set_var("PHILIS_TEST_PDK", &path);
    assert!(Pdk::from_env("PHILIS_TEST_PDK").is_ok());
    assert!(matches!(
        Pdk::from_env("PHILIS_TEST_PDK_MISSING"),
        Err(ApiError::MissingPdkPathEnv(_))
    ));
}

#[test]
fn pdk_from_missing_file_is_io_error() {
    assert!(matches!(
        Pdk::from_json_file("/no/such/philis.json"),
        Err(ApiError::Io(_))
    ));
}

#[test]
fn circuit_from_str_and_file() {
    let c = Circuit::from_spice_str("m1 d g s b nmos\n").unwrap();
    assert_eq!(c.netlist().device_count(), 1);

    let path = temp_file("ckt.sp", "m1 d g s b nmos\nm2 d g s b nmos\n");
    let c2 = Circuit::from_spice_file(&path).unwrap();
    assert_eq!(c2.netlist().device_count(), 2);
}

#[test]
fn subckt_selection() {
    let text = ".subckt inv a y\nm1 y a 0 0 nmos\n.ends\n";
    let c = Circuit::from_spice_str(text).unwrap();
    assert!(c.netlist().subckts().contains(&"inv".to_string()));

    // named selection: present -> ok, absent -> error
    let c = Circuit::from_spice_str(text).unwrap();
    assert!(c.select_subckt("inv").is_ok());
    let c = Circuit::from_spice_str(text).unwrap();
    assert!(matches!(
        c.select_subckt("nope"),
        Err(ApiError::SubcktNotFound(_))
    ));

    // first-subckt promotion when top is empty
    let c = Circuit::from_spice_str(text).unwrap().use_first_subckt_as_top();
    assert!(c.netlist().device_count() >= 1);
}

#[test]
fn empty_netlist_errors_on_run_and_analyze() {
    let c = Circuit::from_spice_str("* only a comment\n").unwrap();
    assert!(matches!(
        c.run(&pdk(), RunConfig::default()),
        Err(ApiError::EmptyNetlist)
    ));
    assert!(matches!(c.analyze(&pdk()), Err(ApiError::EmptyNetlist)));
}

#[test]
fn run_config_default_values() {
    let cfg = RunConfig::default();
    assert_eq!(cfg.row_pitch_nm, 2000);
    assert_eq!(cfg.site_width_nm, 20);
    assert_eq!(cfg.track_pitch_nm, 160);
    assert!(!cfg.strict_constraints);
    assert!(!cfg.fabricatability_mode);
}

#[test]
fn staged_tiers_expose_intermediate_results() {
    let c = Circuit::from_spice_str("m1 d g s b nmos\nm2 d g s b nmos\n").unwrap();
    let constraints = c.analyze(&pdk()).unwrap();
    assert_eq!(constraints.result().honored, 2);

    let placed = constraints.place(RunConfig::default()).unwrap();
    assert!(placed.result().baseline_used);

    let layout = placed.route(RunConfig::default()).unwrap();
    assert_eq!(layout.placed_count(), 2);
}

#[test]
fn api_error_display_is_nonempty() {
    let e = ApiError::EmptyNetlist;
    assert!(!format!("{e}").is_empty());
    assert!(std::error::Error::source(&e).is_none());
}
