//! `philis` CLI — a small, solid front-end over the façade.
//!
//! Two modes:
//! * **Batch** (`philis <netlist-file>`): run the quickstart place & route pass on
//!   a `name x y` file and print the summary. This is the mode `bench/` drives.
//! * **Interactive** (`philis` with no args): ask the user what they want to do —
//!   the quickstart pass, the SPICE flow, or a netlist inspection — reading the
//!   inputs from stdin.

use std::io::{self, Read, Write};
use std::process::ExitCode;

use philis::{Circuit, Pdk, RunConfig};

fn main() -> ExitCode {
    match std::env::args().nth(1) {
        Some(path) => batch(&path),
        None => interactive(),
    }
}

/// Batch mode: read a quickstart netlist file and print the P&R summary.
fn batch(path: &str) -> ExitCode {
    let input = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("philis: cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    match philis::place_and_route(&input) {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("philis: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Interactive mode: a tiny menu driving the façade.
fn interactive() -> ExitCode {
    println!("philis — analog place & route");
    println!("what would you like to do?");
    println!("  1) quickstart place & route   (paste a `name x y` netlist)");
    println!("  2) SPICE flow                 (paste a SPICE netlist, stub PDK)");
    println!("  3) inspect a SPICE netlist    (device & subckt counts)");
    println!("  q) quit");
    print!("> ");
    let _ = io::stdout().flush();

    let choice = match read_line() {
        Some(s) => s.trim().to_string(),
        None => return ExitCode::SUCCESS,
    };

    match choice.as_str() {
        "1" => {
            println!("paste the netlist, then Ctrl-D:");
            let text = read_to_end();
            match philis::place_and_route(&text) {
                Ok(summary) => {
                    println!("{summary}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("philis: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        "2" => {
            println!("paste the SPICE netlist, then Ctrl-D:");
            let text = read_to_end();
            run_spice_flow(&text)
        }
        "3" => {
            println!("paste the SPICE netlist, then Ctrl-D:");
            let text = read_to_end();
            match Circuit::from_spice_str(&text) {
                Ok(c) => {
                    let nl = c.netlist();
                    println!(
                        "devices: {}, subckts: {}",
                        nl.device_count(),
                        nl.subckts().len()
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("philis: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        "q" | "quit" | "" => ExitCode::SUCCESS,
        other => {
            eprintln!("philis: unknown choice {other:?}");
            ExitCode::FAILURE
        }
    }
}

fn run_spice_flow(text: &str) -> ExitCode {
    let pdk = Pdk::from_json_str("{\"tech\":\"stub\"}").expect("stub pdk");
    let circuit = match Circuit::from_spice_str(text) {
        Ok(c) => c.use_first_subckt_as_top(),
        Err(e) => {
            eprintln!("philis: {e}");
            return ExitCode::FAILURE;
        }
    };
    match circuit.run(&pdk, RunConfig::default()) {
        Ok(flow) => {
            println!(
                "constraints honored: {}, placed: {}, routed segments: {}",
                flow.constraints.honored, flow.placement.placed_count, flow.routing.routed_segments
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("philis: {e}");
            ExitCode::FAILURE
        }
    }
}

fn read_line() -> Option<String> {
    let mut s = String::new();
    match io::stdin().read_line(&mut s) {
        Ok(0) => None,
        Ok(_) => Some(s),
        Err(_) => None,
    }
}

fn read_to_end() -> String {
    let mut s = String::new();
    let _ = io::stdin().read_to_string(&mut s);
    s
}
