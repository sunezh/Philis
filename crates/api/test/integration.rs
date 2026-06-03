//! Integration tests for the `philis` library (the default Rust-library face).
//! Run with `cargo test -p api`.

use philis::{parse, place_and_route};

#[test]
fn end_to_end_summary() {
    let netlist = "inv0 0 0\ninv1 10 0\ninv2 10 5\n";
    assert_eq!(
        place_and_route(netlist).unwrap(),
        "placed 3 cells, bbox 10x5, hpwl 15"
    );
}

#[test]
fn parses_full_netlist() {
    let cells = parse("# two cells\na 1 2\nb 3 4\n").unwrap();
    assert_eq!(cells.len(), 2);
}

#[test]
fn rejects_bad_input() {
    assert!(place_and_route("oops 1").is_err());
    assert!(parse("oops 1 two").is_err());
}
