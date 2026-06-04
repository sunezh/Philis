//! The programmatic builder path: construct a circuit in Rust, validate it, and
//! solve it — no SPICE file required.
//!
//! **Design rules embodied here:**
//! * *Immediate mode* — `CircuitDef` accumulates into a value and `build()`
//!   yields an immutable [`BuiltCircuit`]; there is no hidden registry.
//! * *Configuration closures* — `nmos(name, |d| …)` / `net(name, |n| …)` run the
//!   closure once, in-call, and hand control straight back.
//! * *Honest transparency* — every [`Constraint`] is either forwarded to the
//!   engine or surfaced as a [`BuildWarning::UnsupportedConstraint`]; nothing is
//!   silently dropped.

use std::collections::{BTreeMap, BTreeSet};

use crate::core::{Constraint, HintBuilder, Objective, RunConfig};
use crate::io::spice::SpiceNetlist;
use crate::layout::Layout;
use crate::port::{DeviceKind, PortRef};
use crate::units::Length;
use crate::{ApiError, Pdk};

/// Per-device parameters. Each setter returns `&mut Self` for chaining inside the
/// device configuration closure. STUB: values are recorded, not yet used.
#[derive(Debug, Clone, Default)]
pub struct DeviceBuilder {
    kind: Option<DeviceKind>,
}

impl DeviceBuilder {
    pub fn w(&mut self, _w: Length) -> &mut DeviceBuilder {
        self
    }
    pub fn l(&mut self, _l: Length) -> &mut DeviceBuilder {
        self
    }
    pub fn nf(&mut self, _nf: u32) -> &mut DeviceBuilder {
        self
    }
    pub fn r(&mut self, _r: f64) -> &mut DeviceBuilder {
        self
    }
    pub fn c(&mut self, _c: f64) -> &mut DeviceBuilder {
        self
    }
    pub fn area(&mut self, _a: Length) -> &mut DeviceBuilder {
        self
    }
    pub fn pj(&mut self, _pj: Length) -> &mut DeviceBuilder {
        self
    }

    /// The device kind this builder is for.
    pub fn device_kind(&self) -> Option<DeviceKind> {
        self.kind
    }
}

/// Net terminal connections. STUB: connections are recorded for validation.
#[derive(Debug, Clone, Default)]
pub struct NetBuilder {
    connections: Vec<(String, PortRef)>,
}

impl NetBuilder {
    /// Bind a device terminal to this net. Accepts any `Into<PortRef>` — a typed
    /// port (`MosfetPort::Gate`) or one resolved by name.
    pub fn connect(&mut self, device: &str, port: impl Into<PortRef>) -> &mut NetBuilder {
        self.connections.push((device.to_string(), port.into()));
        self
    }

    /// How many terminals are bound to this net.
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }
}

/// An accumulating circuit builder.
#[derive(Debug, Clone, Default)]
pub struct CircuitDef {
    devices: Vec<(String, DeviceKind)>,
    nets: Vec<(String, NetBuilder)>,
    constraints: Vec<Constraint>,
}

impl CircuitDef {
    /// A fresh, empty builder.
    pub fn new() -> CircuitDef {
        CircuitDef::default()
    }

    fn add_device<F>(&mut self, name: &str, kind: DeviceKind, f: F) -> &mut CircuitDef
    where
        F: FnOnce(&mut DeviceBuilder) -> &mut DeviceBuilder,
    {
        let mut b = DeviceBuilder { kind: Some(kind) };
        f(&mut b);
        self.devices.push((name.to_string(), kind));
        self
    }

    pub fn nmos<F>(&mut self, name: &str, f: F) -> &mut CircuitDef
    where
        F: FnOnce(&mut DeviceBuilder) -> &mut DeviceBuilder,
    {
        self.add_device(name, DeviceKind::Nmos, f)
    }
    pub fn pmos<F>(&mut self, name: &str, f: F) -> &mut CircuitDef
    where
        F: FnOnce(&mut DeviceBuilder) -> &mut DeviceBuilder,
    {
        self.add_device(name, DeviceKind::Pmos, f)
    }
    pub fn resistor<F>(&mut self, name: &str, f: F) -> &mut CircuitDef
    where
        F: FnOnce(&mut DeviceBuilder) -> &mut DeviceBuilder,
    {
        self.add_device(name, DeviceKind::Resistor, f)
    }
    pub fn capacitor<F>(&mut self, name: &str, f: F) -> &mut CircuitDef
    where
        F: FnOnce(&mut DeviceBuilder) -> &mut DeviceBuilder,
    {
        self.add_device(name, DeviceKind::Capacitor, f)
    }
    pub fn bjt<F>(&mut self, name: &str, f: F) -> &mut CircuitDef
    where
        F: FnOnce(&mut DeviceBuilder) -> &mut DeviceBuilder,
    {
        self.add_device(name, DeviceKind::Bjt, f)
    }
    pub fn diode<F>(&mut self, name: &str, f: F) -> &mut CircuitDef
    where
        F: FnOnce(&mut DeviceBuilder) -> &mut DeviceBuilder,
    {
        self.add_device(name, DeviceKind::Diode, f)
    }
    pub fn generic<F>(&mut self, name: &str, f: F) -> &mut CircuitDef
    where
        F: FnOnce(&mut DeviceBuilder) -> &mut DeviceBuilder,
    {
        self.add_device(name, DeviceKind::Generic, f)
    }

    /// Declare a net and its terminal connections.
    pub fn net<F>(&mut self, name: &str, f: F) -> &mut CircuitDef
    where
        F: FnOnce(&mut NetBuilder) -> &mut NetBuilder,
    {
        let mut b = NetBuilder::default();
        f(&mut b);
        self.nets.push((name.to_string(), b));
        self
    }

    /// Attach a layout constraint.
    pub fn constrain(&mut self, c: Constraint) -> &mut CircuitDef {
        self.constraints.push(c);
        self
    }

    /// Validate and synthesize the circuit.
    ///
    /// Hard errors: duplicate device, duplicate net, a net connecting an unknown
    /// device, a terminal bound to two nets, a constraint referencing an unknown
    /// device. Warnings: floating nets and unsupported constraints.
    pub fn build(&mut self) -> Result<BuiltCircuit, ApiBuildError> {
        let mut errors = Vec::new();
        let known: BTreeMap<&str, DeviceKind> =
            self.devices.iter().map(|(n, k)| (n.as_str(), *k)).collect();

        let mut seen_dev = BTreeSet::new();
        for (name, _) in &self.devices {
            if !seen_dev.insert(name.as_str()) {
                errors.push(BuildError::DuplicateDevice(name.clone()));
            }
        }
        let mut seen_net = BTreeSet::new();
        for (name, _) in &self.nets {
            if !seen_net.insert(name.as_str()) {
                errors.push(BuildError::DuplicateNet(name.clone()));
            }
        }

        let mut terminal_owner: BTreeMap<(String, u8), String> = BTreeMap::new();
        for (net_name, net) in &self.nets {
            for (dev, port) in &net.connections {
                if !known.contains_key(dev.as_str()) {
                    errors.push(BuildError::UnknownDevice(dev.clone()));
                    continue;
                }
                let key = (dev.clone(), port.idx());
                if let Some(prev) = terminal_owner.get(&key) {
                    if prev != net_name {
                        errors.push(BuildError::TerminalConflict {
                            device: dev.clone(),
                            port: port.name().to_string(),
                            net_a: prev.clone(),
                            net_b: net_name.clone(),
                        });
                    }
                } else {
                    terminal_owner.insert(key, net_name.clone());
                }
            }
        }

        for c in &self.constraints {
            for dref in c.device_refs() {
                if !known.contains_key(dref) {
                    errors.push(BuildError::UnknownDevice(dref.to_string()));
                }
            }
        }

        if !errors.is_empty() {
            return Err(ApiBuildError::Validation(errors));
        }

        let mut warnings = Vec::new();
        for (net_name, net) in &self.nets {
            if net.connection_count() < 2 {
                warnings.push(BuildWarning::FloatingNet(net_name.clone()));
            }
        }
        for c in &self.constraints {
            if !c.is_engine_supported() {
                warnings.push(BuildWarning::UnsupportedConstraint(c.kind()));
            }
        }

        let netlist =
            SpiceNetlist::from_devices(self.devices.iter().map(|(n, _)| n.clone()).collect());
        Ok(BuiltCircuit {
            netlist,
            net_count: self.nets.len(),
            constraints: self.constraints.clone(),
            warnings,
        })
    }
}

/// A validated, immutable circuit ready to solve.
#[derive(Debug, Clone)]
pub struct BuiltCircuit {
    netlist: SpiceNetlist,
    net_count: usize,
    constraints: Vec<Constraint>,
    warnings: Vec<BuildWarning>,
}

impl BuiltCircuit {
    pub fn device_count(&self) -> usize {
        self.netlist.device_count()
    }
    pub fn net_count(&self) -> usize {
        self.net_count
    }
    pub fn constraints(&self) -> &[Constraint] {
        &self.constraints
    }
    pub fn warnings(&self) -> &[BuildWarning] {
        &self.warnings
    }
    pub fn netlist(&self) -> &SpiceNetlist {
        &self.netlist
    }

    /// One-shot solve at an objective preset. STUB engine.
    pub fn solve(&self, _pdk: &Pdk, objective: Objective) -> Result<Layout, ApiError> {
        let cfg = objective.apply_to_config(RunConfig::default());
        crate::flow::solve_netlist(&self.netlist, cfg)
    }

    /// Solve with hints gathered by a configuration closure. STUB: hints are
    /// recorded (see [`HintBuilder`]) but not yet acted on.
    pub fn solve_with<F>(&self, _pdk: &Pdk, objective: Objective, f: F) -> Result<Layout, ApiError>
    where
        F: FnOnce(&mut HintBuilder),
    {
        let mut hints = HintBuilder::new();
        f(&mut hints);
        let mut cfg = objective.apply_to_config(RunConfig::default());
        if let Some(seed) = hints.seed_value() {
            cfg.track_pitch_nm += (seed % 8) as i64; // STUB: coarse perturbation
        }
        crate::flow::solve_netlist(&self.netlist, cfg)
    }
}

/// A build result that also carries its warnings as a value.
#[derive(Debug, Clone)]
pub struct BuildResult {
    pub circuit: BuiltCircuit,
    pub warnings: Vec<BuildWarning>,
}

/// A single validation error.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BuildError {
    DuplicateDevice(String),
    DuplicateNet(String),
    TerminalConflict {
        device: String,
        port: String,
        net_a: String,
        net_b: String,
    },
    UnknownDevice(String),
}

/// The aggregate error returned by [`CircuitDef::build`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiBuildError {
    Validation(Vec<BuildError>),
}

impl ApiBuildError {
    /// The individual validation errors.
    pub fn errors(&self) -> &[BuildError] {
        match self {
            ApiBuildError::Validation(v) => v,
        }
    }
    /// Whether any error matches `pred`.
    pub fn contains<P: Fn(&BuildError) -> bool>(&self, pred: P) -> bool {
        self.errors().iter().any(pred)
    }
}

impl std::fmt::Display for ApiBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "circuit validation failed: {} error(s)",
            self.errors().len()
        )
    }
}
impl std::error::Error for ApiBuildError {}

/// A non-fatal build warning.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BuildWarning {
    /// A net with fewer than two terminals.
    FloatingNet(String),
    /// A constraint the engine recognizes but does not yet honor.
    UnsupportedConstraint(&'static str),
}
