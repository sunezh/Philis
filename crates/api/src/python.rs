//! Python bindings (PyO3), compiled only with `--features extension-module`.
//!
//! This module **re-expresses the Rust façade in Python** rather than
//! reimplementing it: every binding is a one-line shim that calls straight into
//! the `philis` Rust surface, so there is a single source of truth for behavior.
//!
//! The Python surface is a deliberate strict subset — the coarsest tier: load a
//! PDK, load a circuit, run, read counters. The builder, `Layout`, and
//! constraints stay in the Rust crate; a caller who needs them drops down to it.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

use crate::{ApiError, Circuit, Pdk, RunConfig};

fn map_err(e: ApiError) -> PyErr {
    PyValueError::new_err(e.to_string())
}

/// `philis.Pdk` — a loaded process design kit.
#[pyclass(name = "Pdk")]
struct PyPdk {
    inner: Pdk,
}

#[pymethods]
impl PyPdk {
    /// Load a PDK file (JSON or TOML, by extension).
    #[staticmethod]
    fn from_file(path: &str) -> PyResult<PyPdk> {
        Ok(PyPdk {
            inner: Pdk::from_path(path).map_err(map_err)?,
        })
    }

    /// Load a PDK from a path named by an environment variable.
    #[staticmethod]
    fn from_env(var: &str) -> PyResult<PyPdk> {
        Ok(PyPdk {
            inner: Pdk::from_env(var).map_err(map_err)?,
        })
    }

    /// Soft load warnings.
    fn warnings(&self) -> Vec<String> {
        self.inner.warnings().to_vec()
    }
}

/// `philis.Circuit` — a parsed netlist (auto-promotes the first subckt).
#[pyclass(name = "Circuit")]
struct PyCircuit {
    inner: Circuit,
}

#[pymethods]
impl PyCircuit {
    /// Parse a circuit from SPICE text.
    #[staticmethod]
    fn from_spice_text(text: &str) -> PyResult<PyCircuit> {
        Ok(PyCircuit {
            inner: Circuit::from_spice_str(text)
                .map_err(map_err)?
                .use_first_subckt_as_top(),
        })
    }

    /// Parse a circuit from a SPICE file.
    #[staticmethod]
    fn from_spice_file(path: &str) -> PyResult<PyCircuit> {
        Ok(PyCircuit {
            inner: Circuit::from_spice_file(path)
                .map_err(map_err)?
                .use_first_subckt_as_top(),
        })
    }

    /// Run the full flow against a PDK and return read-only counters.
    fn run(&self, pdk: &PyPdk) -> PyResult<PyRunResult> {
        let flow = self
            .inner
            .run(&pdk.inner, RunConfig::default())
            .map_err(map_err)?;
        Ok(PyRunResult {
            device_count: self.inner.netlist().device_count(),
            placed_count: flow.placement.placed_count,
            routed_segments: flow.routing.routed_segments,
        })
    }
}

/// `philis.RunResult` — read-only flow counters.
#[pyclass(name = "RunResult")]
struct PyRunResult {
    #[pyo3(get)]
    device_count: usize,
    #[pyo3(get)]
    placed_count: usize,
    #[pyo3(get)]
    routed_segments: usize,
}

/// The `philis` extension module. `PyInit_philis` must match `module-name` in
/// `pyproject.toml`.
#[pymodule]
fn philis(m: &Bound<'_, PyModule>) -> PyResult<()> {
    /// `philis.place_and_route(netlist: str) -> str` — the quickstart path.
    #[pyfunction]
    fn place_and_route(input: &str) -> PyResult<String> {
        crate::place_and_route(input).map_err(PyValueError::new_err)
    }

    m.add_function(wrap_pyfunction!(place_and_route, m)?)?;
    m.add_class::<PyPdk>()?;
    m.add_class::<PyCircuit>()?;
    m.add_class::<PyRunResult>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
