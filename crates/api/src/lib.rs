//! Philis — analog place & route engine.
//!
//! This crate is compiled into one of three forms, selected at build time:
//!
//! * **Rust library (default):** the `rlib` is linked directly by `bench/` and
//!   by the `philis` binary. No PyO3, no Python — `cargo build` is pure Rust.
//! * **Python extension (`--features extension-module`):** maturin compiles the
//!   `cdylib` with the PyO3 bindings below and packages it as `philis.so`.
//! * **Executable (`--bin philis`):** a thin CLI wrapper, see `main.rs`.
//!
//! The engine logic lives here once; each "face" is just a different entry point.

/// A placed cell: a name and its integer coordinates on the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub name: String,
    pub x: i64,
    pub y: i64,
}

/// Parse a netlist: one whitespace-separated `name x y` triple per line.
/// Blank lines and lines beginning with `#` are ignored.
pub fn parse(input: &str) -> Result<Vec<Cell>, String> {
    let mut cells = Vec::new();
    for (lineno, raw) in input.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        let name = it
            .next()
            .ok_or_else(|| format!("line {}: missing cell name", lineno + 1))?;
        let x = it
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("line {}: missing or invalid x coordinate", lineno + 1))?;
        let y = it
            .next()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| format!("line {}: missing or invalid y coordinate", lineno + 1))?;
        cells.push(Cell {
            name: name.to_string(),
            x,
            y,
        });
    }
    Ok(cells)
}

/// Run the (placeholder) place & route pass and return a one-line summary.
///
/// Computes the bounding box and half-perimeter wirelength (HPWL) of the placed
/// cells — a standard first-order estimate of routing cost.
pub fn place_and_route(input: &str) -> Result<String, String> {
    let cells = parse(input)?;
    if cells.is_empty() {
        return Ok("placed 0 cells".to_string());
    }

    // Bounding box via the shared `utility` generics — one pass per axis, no
    // manual min/max bookkeeping here.
    let (min_x, max_x) = utility::extent!(cells.iter(), |c| c.x).expect("non-empty");
    let (min_y, max_y) = utility::extent!(cells.iter(), |c| c.y).expect("non-empty");

    let (w, h) = (max_x - min_x, max_y - min_y);
    Ok(format!(
        "placed {} cells, bbox {}x{}, hpwl {}",
        cells.len(),
        w,
        h,
        w + h
    ))
}

// ---------------------------------------------------------------------------
// Python bindings — compiled only when the `python` feature is enabled, i.e.
// when maturin builds with `--features extension-module`. A plain `cargo build`
// never sees this module.
// ---------------------------------------------------------------------------
#[cfg(feature = "python")]
mod python {
    use pyo3::exceptions::PyValueError;
    use pyo3::prelude::*;

    /// `philis.place_and_route(netlist: str) -> str`
    #[pyfunction]
    fn place_and_route(input: &str) -> PyResult<String> {
        super::place_and_route(input).map_err(PyValueError::new_err)
    }

    /// The `philis` module, exported as `PyInit_philis`. The function name must
    /// match `module-name` in pyproject.toml.
    #[pymodule]
    fn philis(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_function(wrap_pyfunction!(place_and_route, m)?)?;
        m.add("__version__", env!("CARGO_PKG_VERSION"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_skips_comments_and_blanks() {
        let cells = parse("# header\na 0 0\n\nb 3 4\n").unwrap();
        assert_eq!(cells.len(), 2);
        assert_eq!(
            cells[1],
            Cell {
                name: "b".into(),
                x: 3,
                y: 4
            }
        );
    }

    #[test]
    fn computes_hpwl() {
        let out = place_and_route("a 0 0\nb 3 4").unwrap();
        assert_eq!(out, "placed 2 cells, bbox 3x4, hpwl 7");
    }

    #[test]
    fn empty_input_places_nothing() {
        assert_eq!(place_and_route("\n# nothing\n").unwrap(), "placed 0 cells");
    }

    #[test]
    fn rejects_malformed_lines() {
        assert!(place_and_route("a 1").is_err());
        assert!(place_and_route("a 1 two").is_err());
    }
}
