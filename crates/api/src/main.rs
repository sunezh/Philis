//! `philis` CLI — the Rust executable that `bench/` benchmarks.
//!
//! Reads a netlist from a file argument (or stdin when no argument is given),
//! runs the place & route pass, and prints the summary.

use std::io::Read;
use std::process::ExitCode;

fn main() -> ExitCode {
    let input = match std::env::args().nth(1) {
        Some(path) => match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("philis: cannot read {path}: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => {
            let mut s = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut s) {
                eprintln!("philis: failed to read stdin: {e}");
                return ExitCode::FAILURE;
            }
            s
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
