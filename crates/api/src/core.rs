//! The engine core: everything constraint-, placement-, and routing-dependent.
//!
//! This module is the **engine vocabulary** — the [`Constraint`] taxonomy, the
//! [`Objective`]/[`Strategy`]/[`HintBuilder`] tuning knobs, the [`RunConfig`] and
//! [`FlowResult`] flow types, and the raw stage *oracles*
//! ([`run_constraints`]/[`run_placement`]/[`run_routing`]). It is wired in behind
//! whichever tier drives it: the staged handles in [`crate::flow`] and the
//! programmatic [`crate::builder`] both call these oracles.
//!
//! Everything here is a **STUB** (see `crates/api/STUBS.md`): the types carry the
//! real shape; the bodies do no placement or routing yet.

use crate::io::spice::SpiceNetlist;
use crate::units::{Axis, Coord, Distance, Length};
use crate::ApiError;
// The signoff-first routing contract (Coverage/PdkState/RuleCoverageReport/
// DiagnosticScope/RouteCertificate/RouteDiagnostic/RouteQuality and friends) is
// defined further down in this same module — the placement contract reuses that
// canonical signoff vocabulary, so both live together in `core`.

// ---------------------------------------------------------------------------
// Constraints
// ---------------------------------------------------------------------------

/// A layout constraint. `#[non_exhaustive]` — variants may be added.
///
/// Variants split into two groups: those the engine forwards, and those it does
/// not yet honor. The builder emits a [`crate::BuildWarning::UnsupportedConstraint`]
/// for the latter rather than dropping them silently. See [`Constraint::is_engine_supported`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Constraint {
    // --- forwarded to the engine constraint set ---
    Symmetric(String, String),
    Matching(String, String),
    SelfSymmetric(String),
    SymmetricGroup(Vec<(String, String)>),
    Order(String, String, Axis),
    Align(Vec<String>, Axis),
    DistanceConstraint(String, String, Distance),
    GuardRing(Vec<String>, String),
    NetShield(String),
    SetNetClass(String, String),
    // --- recognized but not yet honored by the engine ---
    SignalFlow(Vec<String>),
    AspectRatio(f64, f64),
    CellBoundary(Distance, Distance),
    FixPosition(String, Coord, Coord),
    PlaceOnGrid(String, Distance),
    PlaceOnBoundary(String, String),
    Group(Vec<String>, String),
    PortLocation(String, String, f64),
    NetMatch(String, String, Distance),
    MultiWire(String, u8),
    DoNotRoute(String),
    ChargeFlow(String),
}

impl Constraint {
    /// A stable kind tag for diagnostics and warnings.
    pub fn kind(&self) -> &'static str {
        match self {
            Constraint::Symmetric(..) => "Symmetric",
            Constraint::Matching(..) => "Matching",
            Constraint::SelfSymmetric(..) => "SelfSymmetric",
            Constraint::SymmetricGroup(..) => "SymmetricGroup",
            Constraint::Order(..) => "Order",
            Constraint::Align(..) => "Align",
            Constraint::DistanceConstraint(..) => "DistanceConstraint",
            Constraint::GuardRing(..) => "GuardRing",
            Constraint::NetShield(..) => "NetShield",
            Constraint::SetNetClass(..) => "SetNetClass",
            Constraint::SignalFlow(..) => "SignalFlow",
            Constraint::AspectRatio(..) => "AspectRatio",
            Constraint::CellBoundary(..) => "CellBoundary",
            Constraint::FixPosition(..) => "FixPosition",
            Constraint::PlaceOnGrid(..) => "PlaceOnGrid",
            Constraint::PlaceOnBoundary(..) => "PlaceOnBoundary",
            Constraint::Group(..) => "Group",
            Constraint::PortLocation(..) => "PortLocation",
            Constraint::NetMatch(..) => "NetMatch",
            Constraint::MultiWire(..) => "MultiWire",
            Constraint::DoNotRoute(..) => "DoNotRoute",
            Constraint::ChargeFlow(..) => "ChargeFlow",
        }
    }

    /// Whether the engine currently forwards this constraint. STUB classification.
    pub fn is_engine_supported(&self) -> bool {
        matches!(
            self,
            Constraint::Symmetric(..)
                | Constraint::Matching(..)
                | Constraint::SelfSymmetric(..)
                | Constraint::SymmetricGroup(..)
                | Constraint::Order(..)
                | Constraint::Align(..)
                | Constraint::DistanceConstraint(..)
                | Constraint::GuardRing(..)
                | Constraint::NetShield(..)
                | Constraint::SetNetClass(..)
        )
    }

    /// Device names this constraint references (for validation).
    pub fn device_refs(&self) -> Vec<&str> {
        match self {
            Constraint::Symmetric(a, b)
            | Constraint::Matching(a, b)
            | Constraint::DistanceConstraint(a, b, _)
            | Constraint::Order(a, b, _)
            | Constraint::NetMatch(a, b, _) => vec![a.as_str(), b.as_str()],
            Constraint::SelfSymmetric(a)
            | Constraint::FixPosition(a, _, _)
            | Constraint::PlaceOnGrid(a, _)
            | Constraint::PlaceOnBoundary(a, _) => vec![a.as_str()],
            Constraint::Align(v, _)
            | Constraint::SignalFlow(v)
            | Constraint::GuardRing(v, _)
            | Constraint::Group(v, _) => v.iter().map(|s| s.as_str()).collect(),
            Constraint::SymmetricGroup(pairs) => pairs
                .iter()
                .flat_map(|(a, b)| [a.as_str(), b.as_str()])
                .collect(),
            _ => Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Objective, Strategy, Hints
// ---------------------------------------------------------------------------

/// What the solver should optimize for. `#[non_exhaustive]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Objective {
    /// Minimize area.
    #[default]
    Area,
    /// Optimize for speed.
    Speed,
    /// Optimize for power.
    Power,
}

impl Objective {
    /// Project this objective onto a [`RunConfig`] preset. STUB pitches.
    pub fn apply_to_config(self, mut cfg: RunConfig) -> RunConfig {
        match self {
            Objective::Area => {
                cfg.row_pitch_nm = 2_000;
                cfg.track_pitch_nm = 160;
            }
            Objective::Speed => {
                cfg.row_pitch_nm = 2_400;
                cfg.track_pitch_nm = 200;
            }
            Objective::Power => {
                cfg.row_pitch_nm = 2_200;
                cfg.track_pitch_nm = 180;
            }
        }
        cfg
    }
}

/// A geometric arrangement style for a group of devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Strategy {
    SideBySide,
    Stacked,
    Mirror,
    Interdigitated,
    CommonCentroid,
    InterdigitatedDummies,
    GuardRingEnclosed,
    Grid,
}

impl Strategy {
    /// A human-readable name for diagnostics.
    pub const fn name(self) -> &'static str {
        match self {
            Strategy::SideBySide => "side-by-side",
            Strategy::Stacked => "stacked",
            Strategy::Mirror => "mirror",
            Strategy::Interdigitated => "interdigitated",
            Strategy::CommonCentroid => "common-centroid",
            Strategy::InterdigitatedDummies => "interdigitated-dummies",
            Strategy::GuardRingEnclosed => "guard-ring-enclosed",
            Strategy::Grid => "grid",
        }
    }
}

/// An accumulating set of solver hints, filled by a configuration closure.
///
/// STUB: hints are recorded but the stub engine does not act on them yet. They
/// are stored (not silently dropped) so the surface stays honest.
#[derive(Debug, Clone, Default)]
pub struct HintBuilder {
    seed: Option<u64>,
    fixed: Vec<(String, Length)>,
    arrangements: Vec<(Vec<String>, Strategy)>,
}

impl HintBuilder {
    pub fn new() -> HintBuilder {
        HintBuilder::default()
    }
    pub fn seed(&mut self, seed: u64) -> &mut HintBuilder {
        self.seed = Some(seed);
        self
    }
    pub fn fix_position(&mut self, device: &str, x: Length, _y: Length) -> &mut HintBuilder {
        self.fixed.push((device.to_string(), x));
        self
    }
    pub fn arrange(&mut self, devices: &[&str], strategy: Strategy) -> &mut HintBuilder {
        self.arrangements
            .push((devices.iter().map(|s| s.to_string()).collect(), strategy));
        self
    }
    pub fn seed_value(&self) -> Option<u64> {
        self.seed
    }
    pub fn fixed_count(&self) -> usize {
        self.fixed.len()
    }
    pub fn arrangement_count(&self) -> usize {
        self.arrangements.len()
    }
}

// ---------------------------------------------------------------------------
// Flow configuration & stage results
// ---------------------------------------------------------------------------

/// Flow tuning knobs shared by the staged and one-shot paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunConfig {
    pub row_pitch_nm: i64,
    pub site_width_nm: i64,
    pub track_pitch_nm: i64,
    pub strict_constraints: bool,
    pub fabricatability_mode: bool,
}

impl Default for RunConfig {
    fn default() -> RunConfig {
        RunConfig {
            row_pitch_nm: 2000,
            site_width_nm: 20,
            track_pitch_nm: 160,
            strict_constraints: false,
            fabricatability_mode: false,
        }
    }
}

/// The three engine stage outputs, kept separate so even the one-shot path
/// exposes the decomposition.
#[derive(Debug, Clone, Default)]
pub struct FlowResult {
    pub constraints: ConstraintResult,
    pub placement: PlacementResult,
    pub routing: RoutingResult,
}

/// Input to [`run_constraints`].
#[derive(Debug, Clone, Default)]
pub struct ConstraintRunInput {
    pub device_count: usize,
}

/// Input to [`run_placement`].
#[derive(Debug, Clone, Default)]
pub struct PlacementRunInput {
    pub device_count: usize,
}

/// Input to [`run_routing`]. `baseline_used` is threaded from the placement
/// result — the staged façade does this for the caller.
#[derive(Debug, Clone, Default)]
pub struct RoutingRunInput {
    pub net_count: usize,
    pub baseline_used: bool,
}

/// Output of [`run_constraints`].
#[derive(Debug, Clone, Default)]
pub struct ConstraintResult {
    pub honored: usize,
    pub unsupported: usize,
}

/// Output of [`run_placement`].
#[derive(Debug, Clone, Default)]
pub struct PlacementResult {
    pub placed_count: usize,
    /// Whether the placer fell back to its baseline pass — needed by routing.
    pub baseline_used: bool,
    /// The machine-checkable evidence the placer emits alongside geometry — the
    /// lexicographic acceptance-gate verdicts and the router-handoff witness.
    /// STUB: a *degraded* certificate (every gate `Degraded`); see
    /// [`PlacementCertificate`] and `crates/api/STUBS.md`.
    pub certificate: PlacementCertificate,
}

/// Output of [`run_routing`] — Philis's `RouteSolution`. The router returns
/// geometry (here summarized as [`RoutingResult::routed_segments`]) **plus** a
/// machine-checkable [`RouteCertificate`] and a [`RouteQuality`] facts record,
/// never geometry alone. `diagnostics` carries the typed failure/infeasibility
/// classes. See [`crate::core`] and `docs/Developer/route/`.
#[derive(Debug, Clone, Default)]
pub struct RoutingResult {
    /// Count of emitted routed segments (the geometry summary).
    pub routed_segments: usize,
    /// Signoff and parasitic facts reported directly (not a scalar score).
    pub quality: RouteQuality,
    /// The route certificate: lexicographic hard vector, rule coverage, per-gate
    /// evidence, degraded-confidence markers, and any infeasible core.
    pub certificate: RouteCertificate,
    /// Typed diagnoses (empty on a clean run).
    pub diagnostics: Vec<RouteDiagnostic>,
}

/// Stage oracle: compile constraints. STUB.
pub fn run_constraints(input: &ConstraintRunInput) -> ConstraintResult {
    ConstraintResult {
        honored: input.device_count,
        unsupported: 0,
    }
}

/// Stage oracle: place. STUB.
pub fn run_placement(input: &PlacementRunInput) -> PlacementResult {
    PlacementResult {
        placed_count: input.device_count,
        baseline_used: true,
        certificate: PlacementCertificate::stub(),
    }
}

/// Stage oracle: route. STUB.
///
/// Returns no geometry (`routed_segments = 0`) and the honest stub certificate
/// ([`RouteCertificate::stub`]): feasible-*shaped* (nothing is proven wrong) but
/// explicitly `degraded_confidence`, so a stub result can never be mistaken for a
/// signoff-quality route. `quality` likewise carries the conservative
/// degraded-confidence marker. No real routing, verification, or extraction runs.
pub fn run_routing(_input: &RoutingRunInput) -> RoutingResult {
    RoutingResult {
        routed_segments: 0,
        quality: RouteQuality {
            degraded_confidence: true,
            ..RouteQuality::default()
        },
        certificate: RouteCertificate::stub(),
        diagnostics: Vec::new(),
    }
}

/// Derive a [`ConstraintRunInput`] from a parsed netlist.
pub fn constraint_input(netlist: &SpiceNetlist) -> ConstraintRunInput {
    ConstraintRunInput {
        device_count: netlist.device_count(),
    }
}

/// Run all three stages over a netlist's hypergraph. Shared by the one-shot
/// [`crate::Circuit::run`] and [`crate::BuiltCircuit::solve`].
pub fn run_flow(netlist: &SpiceNetlist, _cfg: RunConfig) -> Result<FlowResult, ApiError> {
    if netlist.device_count() == 0 {
        return Err(ApiError::EmptyNetlist);
    }
    // The engine consumes the netlist as a bipartite hypergraph; that type will
    // be exposed by `io::spice` (deferred — see `crates/api/STUBS.md`).
    let constraints = run_constraints(&ConstraintRunInput {
        device_count: netlist.device_count(),
    });
    let placement = run_placement(&PlacementRunInput {
        device_count: netlist.device_count(),
    });
    let routing = run_routing(&RoutingRunInput {
        net_count: netlist.device_count(),
        baseline_used: placement.baseline_used,
    });
    Ok(FlowResult {
        constraints,
        placement,
        routing,
    })
}

// ===========================================================================
// Signoff-first placement contract
//
// The placement sibling of [`crate::core`]. The placer returns geometry *plus*
// a machine-checkable certificate, never geometry alone, and acceptance is a
// lexicographic hierarchy of hard *gates* — technology coverage → local DRC →
// device equivalence → terminal access → routability → intent satisfaction —
// gated before any soft objective (area, HPWL, parasitics). It shares routing's
// canonical [`Coverage`]/[`PdkState`]/[`RuleCoverageReport`] vocabulary.
//
// Shape-real, behavior-STUB: the types are the real contract and are
// unit-tested; the gate *evaluator* is the engine work still to come, so
// [`run_placement`] emits [`PlacementCertificate::stub`] (every gate
// `Degraded`). See `docs/Developer/place/` and `crates/api/STUBS.md`.
// ===========================================================================

/// How strongly the engine must honor an analog-intent obligation.
///
/// The canonical `ObligationClass` owned by the constraints layer has seven
/// members (`Hard | Joint | Bounded | Soft | Unsupported | ExternalOnly |
/// ManualReview`); the realized engine encodes the four below.
/// [`Constraint::intent_class`] maps the [`Constraint`] taxonomy onto them.
/// `Unsupported` is exactly the [`crate::BuildWarning::UnsupportedConstraint`]
/// set — surfaced, never silently dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IntentClass {
    /// A violation makes the placement unacceptable (a gate failure).
    Hard,
    /// Admissible only inside an explicit numerical tolerance.
    Bounded,
    /// Optimized only after every hard gate passes.
    Soft,
    /// Refused or reported degraded before solving.
    Unsupported,
}

impl IntentClass {
    /// A stable lowercase tag for diagnostics and reports.
    pub const fn tag(self) -> &'static str {
        match self {
            IntentClass::Hard => "hard",
            IntentClass::Bounded => "bounded",
            IntentClass::Soft => "soft",
            IntentClass::Unsupported => "unsupported",
        }
    }

    /// Whether this obligation is a *gate* — a violation rejects the placement
    /// rather than merely worsening a soft score. Only `Hard`.
    pub const fn is_gate(self) -> bool {
        matches!(self, IntentClass::Hard)
    }
}

impl Constraint {
    /// The obligation class this constraint lowers to in the placement intent
    /// graph. Engine-unsupported variants ([`Constraint::is_engine_supported`])
    /// map to [`IntentClass::Unsupported`] — never silently dropped.
    pub fn intent_class(&self) -> IntentClass {
        match self {
            // Hard analog intent — a violation is electrically wrong.
            Constraint::Symmetric(..)
            | Constraint::SelfSymmetric(..)
            | Constraint::SymmetricGroup(..)
            | Constraint::Order(..)
            | Constraint::Align(..)
            | Constraint::GuardRing(..) => IntentClass::Hard,
            // Bounded — admissible only inside an explicit numerical tolerance.
            Constraint::Matching(..) | Constraint::DistanceConstraint(..) => IntentClass::Bounded,
            // Soft — optimized only after the gates pass.
            Constraint::NetShield(..) | Constraint::SetNetClass(..) => IntentClass::Soft,
            // Everything the engine does not yet forward.
            _ => IntentClass::Unsupported,
        }
    }
}

/// One lexicographic acceptance gate. Acceptance requires *every* gate to hold;
/// the order governs which violation is blamed first and which costs are
/// computed, not whether an accepted result is sound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AcceptanceGate {
    /// G0 — required PDK / host-template / rule coverage exists.
    TechnologyCoverage,
    /// G2 — placement-owned local DRC, non-overlap, host-outline legality.
    LocalDrc,
    /// G1 — every device and imported cell is LVS-equivalent.
    DeviceEquivalence,
    /// G4a — every required terminal has a non-empty, typed access set.
    TerminalAccess,
    /// G4b — channels carry enough capacity for a router to have a feasible path.
    Routability,
    /// G3 — all hard analog intent constraints hold (or are diagnosed).
    IntentSatisfaction,
}

/// The gates in the order the engine evaluates them — the *realized acceptance*
/// order, which deliberately differs from the *design-rationale* order
/// (`coverage → equivalence → DRC → intent → access`): `LocalDrc` runs before
/// `DeviceEquivalence` (geometry already exists, and local DRC is the cheapest
/// high-yield rejector; equivalence is invariant under legalization), and
/// `IntentSatisfaction` runs last (it is the gate that is repairable before
/// legalization). See `docs/Developer/place/Signoff-First-Placement.html`.
pub const ACCEPTANCE_GATE_ORDER: [AcceptanceGate; 6] = [
    AcceptanceGate::TechnologyCoverage,
    AcceptanceGate::LocalDrc,
    AcceptanceGate::DeviceEquivalence,
    AcceptanceGate::TerminalAccess,
    AcceptanceGate::Routability,
    AcceptanceGate::IntentSatisfaction,
];

impl AcceptanceGate {
    /// The gate name, exactly as it appears in the certificate.
    pub const fn name(self) -> &'static str {
        match self {
            AcceptanceGate::TechnologyCoverage => "TechnologyCoverage",
            AcceptanceGate::LocalDrc => "LocalDrc",
            AcceptanceGate::DeviceEquivalence => "DeviceEquivalence",
            AcceptanceGate::TerminalAccess => "TerminalAccess",
            AcceptanceGate::Routability => "Routability",
            AcceptanceGate::IntentSatisfaction => "IntentSatisfaction",
        }
    }

    /// Whether group-preserving legalization may repair a *pre-legalization*
    /// failure of this gate. Only `LocalDrc` and `IntentSatisfaction` are
    /// repairable: legalization can re-snap geometry to the lattice and
    /// re-mirror a symmetry pair, but it cannot conjure a missing rule deck,
    /// device generator, terminal-access set, or channel capacity — so the other
    /// four gates fail terminally and produce a conflict core.
    pub const fn is_repairable(self) -> bool {
        matches!(
            self,
            AcceptanceGate::LocalDrc | AcceptanceGate::IntentSatisfaction
        )
    }
}

/// The verdict of one gate on one candidate snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum GateStatus {
    /// The gate's predicates all hold on coded evidence.
    Pass,
    /// The gate holds only on `ExternalOnly`/`Approximate`/`Missing` evidence —
    /// useful for exploration, never a DRC=0/LVS=true claim. The default,
    /// because a gate with no evidence is degraded, not passing.
    #[default]
    Degraded,
    /// A hard predicate is violated.
    Fail,
}

/// One gate's evaluation: the gate, its status, and a human-readable evidence
/// string. Evidence strings are sorted into the certificate for determinism.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateEvaluation {
    pub gate: AcceptanceGate,
    pub status: GateStatus,
    pub evidence: String,
}

/// Decide acceptance from a gate vector under the repairable-failure policy.
///
/// Called twice by the pipeline: once **pre-legalization** with
/// `allow_repairable_failures = true` (a `Fail` on a repairable gate is recorded
/// as repair-pending, not a reject), and once **post-legalization** with `false`
/// (any `Fail` is a hard reject). [`GateStatus::Degraded`] never rejects; it is
/// surfaced as a [`DegradedConfidenceCause`].
pub fn evaluate_gate_acceptance(gates: &[GateEvaluation], allow_repairable_failures: bool) -> bool {
    gates.iter().all(|g| match g.status {
        GateStatus::Pass | GateStatus::Degraded => true,
        GateStatus::Fail => allow_repairable_failures && g.gate.is_repairable(),
    })
}

/// Why a result is degraded-confidence rather than signoff-quality: the gate
/// that degraded, the reason, and the coverage class of the missing evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DegradedConfidenceCause {
    pub gate: AcceptanceGate,
    pub reason: String,
    /// The coverage class of the evidence that was unavailable.
    pub coverage: Coverage,
}

/// The class of a placement conflict. Each class implies a different repair
/// scope and a different blamed gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConflictClass {
    /// A PDK rule predicate is violated (spacing, enclosure, grid, well/tap…).
    #[default]
    PdkRule,
    /// A device or imported cell could not be generated LVS-equivalent.
    DeviceEquivalence,
    /// A hard analog-intent obligation is unsatisfied (symmetry, order, centroid).
    IntentViolation,
    /// A required terminal has no legal access candidate.
    TerminalAccess,
    /// A channel or bin lacks the capacity a router would need.
    Capacity,
    /// A PEX/EM/noise/LDE budget is exceeded (today an external/missing oracle).
    ParasiticBudget,
    /// A required rule class is absent from the compiled PDK.
    CoverageGap,
    /// A fixed host-interface object would have to be altered.
    HostInterface,
}

impl ConflictClass {
    /// The acceptance gate this conflict is blamed against.
    pub const fn gate(self) -> AcceptanceGate {
        match self {
            ConflictClass::CoverageGap => AcceptanceGate::TechnologyCoverage,
            ConflictClass::PdkRule | ConflictClass::HostInterface => AcceptanceGate::LocalDrc,
            ConflictClass::DeviceEquivalence => AcceptanceGate::DeviceEquivalence,
            ConflictClass::TerminalAccess => AcceptanceGate::TerminalAccess,
            ConflictClass::Capacity | ConflictClass::ParasiticBudget => AcceptanceGate::Routability,
            ConflictClass::IntentViolation => AcceptanceGate::IntentSatisfaction,
        }
    }
}

/// One typed placement conflict. For repairable cases it carries the smallest
/// implicated group and the failed predicate set; for infeasible cases it is
/// part of the preserved unsatisfied core. The placement analog of
/// [`crate::core::RouteDiagnostic`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlacementConflict {
    pub class: ConflictClass,
    pub scope: DiagnosticScope,
    /// A human-readable rendering.
    pub message: String,
    /// The implicated instances or groups (the minimal repair seed).
    pub subjects: Vec<String>,
    /// The violated predicate / rule / intent IDs.
    pub predicates: Vec<String>,
}

/// The placement **readiness witness** handed to the router — the evidence that
/// a route toward DRC=0/LVS=true is *reachable*, not the routed result itself.
/// It is the input [`crate::core`]'s terminal-access and capacity oracles
/// consume.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouterHandoffWitness {
    /// Whether every terminal has at least one access candidate recorded.
    pub has_access_map: bool,
    /// Channels reserved for matched/shielded/critical nets.
    pub reserved_channel_count: usize,
    /// Schematic terminals with a typed access set.
    pub terminal_count: usize,
    /// Fixed host-interface pins respected as boundary conditions.
    pub host_pin_count: usize,
    /// Obstruction snapshots handed across (a hash count for stability).
    pub obstruction_count: usize,
    /// A human-readable note (e.g. the degraded reason on a stub).
    pub note: String,
}

/// The machine-checkable **placement certificate**: geometry's companion proof.
/// Every gate carries a verdict and evidence, an infeasible problem preserves
/// its conflict core, and the router-handoff witness travels with it.
///
/// A `PlacementCertificate` proves only facts owned by placement and checked by
/// declared oracles — it is **not** a foundry-signoff substitute. Final DRC=0
/// and LVS=true belong to verification ([`crate::verify`]); placement emits
/// *readiness witnesses*. The placement sibling of
/// [`crate::core::RouteCertificate`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlacementCertificate {
    /// Hash of the placement problem `(T, S, H, M, C, O, B)`.
    pub problem_hash: u64,
    /// Hash of the compiled PDK placement oracle.
    pub pdk_oracle_hash: u64,
    /// The classified PDK readiness state (shared with routing).
    pub pdk_state: PdkState,
    /// The per-gate verdicts, in [`ACCEPTANCE_GATE_ORDER`].
    pub gates: Vec<GateEvaluation>,
    /// Per-class rule coverage of the compiled technology.
    pub coverage: RuleCoverageReport,
    /// Set when a required rule/model/deck was external/approximate/missing —
    /// signoff-ready *shaped*, not signoff-quality.
    pub degraded_confidence: bool,
    /// The structured degraded-confidence causes.
    pub degraded_causes: Vec<DegradedConfidenceCause>,
    /// The router-handoff witness (placement → routing contract).
    pub handoff: RouterHandoffWitness,
    /// The preserved unsatisfied core / minimized conflict set for failures.
    pub infeasible_core: Vec<PlacementConflict>,
    /// The solver path taken, e.g. `"exact->analytical->legalize"`.
    pub solver_path: String,
    /// The replay seed — placement is deterministic in its seed.
    pub seed: Option<u64>,
}

impl PlacementCertificate {
    /// True iff every gate passed on coded evidence *and* the result is not
    /// degraded — a genuine signoff-quality (readiness) claim.
    pub fn is_signoff_quality(&self) -> bool {
        !self.degraded_confidence
            && !self.gates.is_empty()
            && self
                .gates
                .iter()
                .all(|g| matches!(g.status, GateStatus::Pass))
    }

    /// True iff no gate hard-failed — the result is in the acceptable region (it
    /// may still be degraded-confidence). Equivalent to a post-legalization
    /// [`evaluate_gate_acceptance`] with no repairable allowance.
    pub fn is_accepted(&self) -> bool {
        evaluate_gate_acceptance(&self.gates, false)
    }

    /// The honest stub certificate: feasible-*shaped* but explicitly
    /// degraded-confidence (every gate [`GateStatus::Degraded`]) with the
    /// conservative [`PdkState::SparseResearch`] state, so a stub result can
    /// never be mistaken for a proven placement. This is the shape a real run
    /// fills in; only the verdicts are placeholders.
    pub fn stub() -> PlacementCertificate {
        let gates = ACCEPTANCE_GATE_ORDER
            .iter()
            .map(|&gate| GateEvaluation {
                gate,
                status: GateStatus::Degraded,
                evidence: format!("{}: engine stub — gate not evaluated", gate.name()),
            })
            .collect();
        let degraded_causes = ACCEPTANCE_GATE_ORDER
            .iter()
            .map(|&gate| DegradedConfidenceCause {
                gate,
                reason: "placement engine is a stub; no oracle evidence".to_string(),
                coverage: Coverage::Missing,
            })
            .collect();
        PlacementCertificate {
            pdk_state: PdkState::SparseResearch,
            gates,
            degraded_confidence: true,
            degraded_causes,
            handoff: RouterHandoffWitness {
                note: "engine stub — no access map".to_string(),
                ..RouterHandoffWitness::default()
            },
            solver_path: "baseline (stub)".to_string(),
            ..PlacementCertificate::default()
        }
    }
}

// The flat oracle sub-namespaces re-exported at the crate root and under
// `philis::next::*`, mirroring the eventual split into separate engine crates.
pub mod constraints {
    pub use super::{run_constraints, ConstraintResult, ConstraintRunInput};
}
pub mod placer {
    pub use super::{run_placement, PlacementResult, PlacementRunInput};
    // The signoff-first placement contract (the typed certificate vocabulary).
    pub use super::{
        evaluate_gate_acceptance, AcceptanceGate, ConflictClass, DegradedConfidenceCause,
        GateEvaluation, GateStatus, IntentClass, PlacementCertificate, PlacementConflict,
        RouterHandoffWitness, ACCEPTANCE_GATE_ORDER,
    };
}
pub mod router {
    pub use super::{run_routing, RoutingResult, RoutingRunInput};
    // The signoff-first routing contract (the typed certificate vocabulary).
    pub use crate::core::{
        AccessConfidence, Coverage, DiagnosticClass, DiagnosticScope, Evidence, HardVector,
        NetClass, PdkState, RouteCertificate, RouteDiagnostic, RouteQuality, RuleCoverageReport,
    };
}

#[cfg(test)]
mod placement_tests {
    use super::*;
    use crate::units::{Axis, Coord};

    #[test]
    fn acceptance_gate_order_is_the_realized_order() {
        // G0 first, intent last; LocalDrc deliberately precedes DeviceEquivalence.
        assert_eq!(ACCEPTANCE_GATE_ORDER[0], AcceptanceGate::TechnologyCoverage);
        assert_eq!(ACCEPTANCE_GATE_ORDER[5], AcceptanceGate::IntentSatisfaction);
        let drc = ACCEPTANCE_GATE_ORDER
            .iter()
            .position(|g| *g == AcceptanceGate::LocalDrc)
            .unwrap();
        let equiv = ACCEPTANCE_GATE_ORDER
            .iter()
            .position(|g| *g == AcceptanceGate::DeviceEquivalence)
            .unwrap();
        assert!(drc < equiv);
    }

    #[test]
    fn only_local_drc_and_intent_are_repairable() {
        assert!(AcceptanceGate::LocalDrc.is_repairable());
        assert!(AcceptanceGate::IntentSatisfaction.is_repairable());
        assert!(!AcceptanceGate::TechnologyCoverage.is_repairable());
        assert!(!AcceptanceGate::DeviceEquivalence.is_repairable());
        assert!(!AcceptanceGate::TerminalAccess.is_repairable());
        assert!(!AcceptanceGate::Routability.is_repairable());
    }

    #[test]
    fn repairable_failure_policy() {
        let fail = |gate| GateEvaluation {
            gate,
            status: GateStatus::Fail,
            evidence: String::new(),
        };
        // A repairable gate failing is accepted pre-legalization, rejected after.
        let local = [fail(AcceptanceGate::LocalDrc)];
        assert!(evaluate_gate_acceptance(&local, true));
        assert!(!evaluate_gate_acceptance(&local, false));
        // A non-repairable gate failing is rejected in both modes.
        let access = [fail(AcceptanceGate::TerminalAccess)];
        assert!(!evaluate_gate_acceptance(&access, true));
        assert!(!evaluate_gate_acceptance(&access, false));
    }

    #[test]
    fn stub_certificate_is_honest() {
        let cert = PlacementCertificate::stub();
        // Feasible-shaped (no hard fail) but never signoff-quality.
        assert!(cert.is_accepted());
        assert!(!cert.is_signoff_quality());
        assert!(cert.degraded_confidence);
        assert_eq!(cert.pdk_state, PdkState::SparseResearch);
        assert_eq!(cert.gates.len(), ACCEPTANCE_GATE_ORDER.len());
        assert!(cert.gates.iter().all(|g| g.status == GateStatus::Degraded));
    }

    #[test]
    fn run_placement_emits_the_degraded_stub_certificate() {
        let p = run_placement(&PlacementRunInput { device_count: 3 });
        assert_eq!(p.placed_count, 3);
        assert!(!p.certificate.is_signoff_quality());
        assert!(p.certificate.degraded_confidence);
    }

    #[test]
    fn intent_class_maps_the_constraint_taxonomy() {
        assert_eq!(
            Constraint::Symmetric("a".into(), "b".into()).intent_class(),
            IntentClass::Hard
        );
        assert_eq!(
            Constraint::Matching("a".into(), "b".into()).intent_class(),
            IntentClass::Bounded
        );
        assert_eq!(
            Constraint::NetShield("n".into()).intent_class(),
            IntentClass::Soft
        );
        // An engine-unsupported variant lowers to Unsupported, never dropped.
        let unsupported = Constraint::FixPosition("a".into(), Coord::new(0), Coord::new(0));
        assert!(!unsupported.is_engine_supported());
        assert_eq!(unsupported.intent_class(), IntentClass::Unsupported);
        assert_eq!(
            Constraint::Align(vec!["a".into()], Axis::X).intent_class(),
            IntentClass::Hard
        );
    }

    #[test]
    fn conflict_class_is_blamed_against_the_right_gate() {
        assert_eq!(
            ConflictClass::CoverageGap.gate(),
            AcceptanceGate::TechnologyCoverage
        );
        assert_eq!(
            ConflictClass::TerminalAccess.gate(),
            AcceptanceGate::TerminalAccess
        );
        assert_eq!(
            ConflictClass::IntentViolation.gate(),
            AcceptanceGate::IntentSatisfaction
        );
    }
}

// ===========================================================================
// Routing contract (folded into core) — the signoff-first certificate vocabulary
// ===========================================================================
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Canonical coverage & PDK state
// ---------------------------------------------------------------------------

/// The canonical rule-coverage class for a single PDK predicate.
///
/// Open and sparse PDKs are not merely lower-confidence — they contain rules
/// that are explicitly *not coded*, rules caught *only by LVS*, and rules that
/// must be *manually reviewed* before tapeout. The certificate reports coverage,
/// not just DRC pass/fail, so "internal evidence without required decks" reads
/// as **signoff-ready, not signoff-quality**.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Coverage {
    /// Fully encoded as an internal predicate.
    #[default]
    Coded,
    /// Encoded, but only for a subset of its dependencies.
    Partial,
    /// Modeled by a calibrated surrogate, not an exact rule.
    Approximate,
    /// Enforced only by an external engine (e.g. LVS — see [`Coverage::ExternalOnly`] usage).
    ExternalOnly,
    /// Must be checked by a human before tapeout.
    ManualReview,
    /// A guideline, not a hard rule (not in the DRC runset).
    RecommendedOnly,
    /// Known to exist for this node but unsupported by the compiler.
    Unsupported,
    /// A required rule class that is absent from the PDK.
    Missing,
    /// The PDK states two mutually inconsistent rules.
    Contradictory,
}

impl Coverage {
    /// A stable lowercase tag for diagnostics and reports.
    pub const fn tag(self) -> &'static str {
        match self {
            Coverage::Coded => "coded",
            Coverage::Partial => "partial",
            Coverage::Approximate => "approximate",
            Coverage::ExternalOnly => "external-only",
            Coverage::ManualReview => "manual-review",
            Coverage::RecommendedOnly => "recommended-only",
            Coverage::Unsupported => "unsupported",
            Coverage::Missing => "missing",
            Coverage::Contradictory => "contradictory",
        }
    }
}

/// The readiness state of a whole compiled PDK, classified from the per-predicate
/// [`Coverage`] buckets plus structural sanity of layers/tracks/vias.
///
/// The classifier is deliberately conservative: a single [`Coverage::Missing`]
/// required class or structural defect promotes the whole PDK to
/// [`PdkState::ContradictoryOrMissing`], and any [`Coverage::Unsupported`] /
/// [`Coverage::ManualReview`] rule prevents a [`PdkState::FullSignoff`] claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum PdkState {
    /// All required classes `Coded`; no `ManualReview`/`Unsupported` gaps. Use
    /// exact rules and external final signoff.
    FullSignoff,
    /// Required classes present, some `Partial`/`ExternalOnly`. Use internal
    /// predicates; flag signoff confidence as abstract.
    AbstractComplete,
    /// `Unsupported`/`ManualReview` rules dominate. Route only under known rules;
    /// return a degraded-confidence certificate.
    ///
    /// This is the default for a stub: no real PDK has been compiled, so the most
    /// honest state is "we route only what we can prove, with low confidence".
    #[default]
    SparseResearch,
    /// `Contradictory`/`Missing` required classes or broken structure. Refuse the
    /// affected route classes and report the unsatisfied PDK fields.
    ContradictoryOrMissing,
}

// ---------------------------------------------------------------------------
// Net class & access confidence
// ---------------------------------------------------------------------------

/// The analog intent class of a net. Drives which obligations are hard vs.
/// bounded and how the net is routed (jointly vs. independently). See the
/// net-class obligation table in the routing docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NetClass {
    /// Single-ended ordinary net: access, connectivity, DRC, basic parasitics.
    #[default]
    Ordinary,
    /// Routed before ordinary nets; bounded R/Cg/detour.
    Critical,
    /// Shield/spacing legality; bounded coupling and noise.
    Sensitive,
    /// Routed jointly with its partner; equal-topology, bounded skew/asymmetry.
    DifferentialPair,
    /// Mirrored access across a preserved symmetry axis.
    SelfSymmetric,
    /// Common-centroid / interdigitated array routed jointly.
    MatchedArray,
    /// Equal length / per-layer length / via count / topology as a hard group.
    ExactMatch,
    /// Wider trunks, multi-via; EM/IR hard when declared; domain separation.
    PowerGround,
    /// Shield continuity, return path, bounded bends/vias; calibrated PEX.
    RfMmwave,
    /// Voltage-domain spacing, well isolation, thick-oxide markers.
    HighVoltage,
    /// Required connectivity, low-impedance return, spacing (LVS/ERC/latch-up).
    GuardSubstrate,
}

impl NetClass {
    /// Whether nets of this class must be rerouted *jointly* with their group —
    /// the group-preserving invariant forbids independent reroute.
    pub const fn is_grouped(self) -> bool {
        matches!(
            self,
            NetClass::DifferentialPair
                | NetClass::SelfSymmetric
                | NetClass::MatchedArray
                | NetClass::ExactMatch
        )
    }
}

/// How trustworthy a terminal-access candidate is. A heuristic-access route is
/// *not* a signoff-quality answer until verified against exact device geometry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AccessConfidence {
    /// Exact pin shape from the PDK / imported cell.
    ExactPin,
    /// Pin geometry from a device generator.
    GeneratedPin,
    /// Pin inferred from partial placement data.
    InferredPin,
    /// A heuristic anchor — placement exposed no orientation/exact geometry.
    /// The most conservative default.
    #[default]
    HeuristicPin,
}

// ---------------------------------------------------------------------------
// Lexicographic hard vector
// ---------------------------------------------------------------------------

/// The lexicographic **hard-feasibility vector**. Every component must reach zero
/// (and `invalid_pdk` be false) before *any* soft objective is compared. This is
/// the spine of the acceptance relation: a nonzero component is a *different
/// feasibility class*, not a worse score, so no parasitic improvement can redeem
/// it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HardVector {
    /// The PDK is contradictory/missing for a required route class.
    pub invalid_pdk: bool,
    /// Required terminals with no legal access candidate.
    pub missing_access: usize,
    /// DRC predicate violations.
    pub drc_violations: usize,
    /// LVS opens/shorts/mismatches on required nets.
    pub lvs_mismatches: usize,
    /// Required nets not yet routed.
    pub unrouted_required_nets: usize,
    /// Antenna/EM errors promoted to hard by PDK or reliability policy.
    pub unresolved_em_antenna_hard_errors: usize,
}

impl HardVector {
    /// True iff the route is in the feasible region (all hard levels satisfied).
    pub fn is_feasible(&self) -> bool {
        !self.invalid_pdk
            && self.missing_access == 0
            && self.drc_violations == 0
            && self.lvs_mismatches == 0
            && self.unrouted_required_nets == 0
            && self.unresolved_em_antenna_hard_errors == 0
    }

    /// The vector as a tuple in lexicographic priority order, for ranking two
    /// candidates. `invalid_pdk` ranks first as a `usize` (0/1).
    pub fn as_priority(&self) -> [usize; 6] {
        [
            self.invalid_pdk as usize,
            self.missing_access,
            self.drc_violations,
            self.lvs_mismatches,
            self.unrouted_required_nets,
            self.unresolved_em_antenna_hard_errors,
        ]
    }

    /// Lexicographic "A is at least as feasible as B".
    pub fn lex_le(&self, other: &HardVector) -> bool {
        self.as_priority() <= other.as_priority()
    }
}

// ---------------------------------------------------------------------------
// Evidence & coverage report
// ---------------------------------------------------------------------------

/// The proof form a hard gate carries in the certificate. Every hard gate must
/// carry exactly one of these — the difference between an auditable result and an
/// anecdotal one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Evidence {
    /// An internal solver/oracle proof artifact.
    #[default]
    InternalProof,
    /// An external signoff report, recorded by content hash / path.
    ExternalReport { hash: String },
    /// A solver counterexample (the gate failed; this reproduces it).
    Counterexample { detail: String },
}

/// Per-class counts of rule coverage. Field names keep the legacy spelling used
/// across the toolchain; the mapping to the canonical [`Coverage`] enum is:
/// `partially_coded`→`Partial`, `lvs_only`→`ExternalOnly` (domain `LVS`),
/// `signoff_only`→`ExternalOnly`, `manual_review`→`ManualReview`,
/// `unsupported`→`Unsupported`, `missing`→`Missing`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RuleCoverageReport {
    pub coded: usize,
    pub partially_coded: usize,
    pub approximate: usize,
    pub lvs_only: usize,
    pub signoff_only: usize,
    pub recommended_only: usize,
    pub manual_review: usize,
    pub unsupported: usize,
    pub missing: usize,
    pub contradictory: usize,
}

impl RuleCoverageReport {
    /// The count for a canonical [`Coverage`] class. `ExternalOnly` returns the
    /// sum of the `lvs_only` and `signoff_only` legacy buckets.
    pub fn count(&self, coverage: Coverage) -> usize {
        match coverage {
            Coverage::Coded => self.coded,
            Coverage::Partial => self.partially_coded,
            Coverage::Approximate => self.approximate,
            Coverage::ExternalOnly => self.lvs_only + self.signoff_only,
            Coverage::ManualReview => self.manual_review,
            Coverage::RecommendedOnly => self.recommended_only,
            Coverage::Unsupported => self.unsupported,
            Coverage::Missing => self.missing,
            Coverage::Contradictory => self.contradictory,
        }
    }

    /// A canonical view as a `Coverage → count` map (the future-proof shape that
    /// also carries `Approximate`/`RecommendedOnly`/`Contradictory`).
    pub fn as_map(&self) -> BTreeMap<&'static str, usize> {
        let mut m = BTreeMap::new();
        for c in [
            Coverage::Coded,
            Coverage::Partial,
            Coverage::Approximate,
            Coverage::ExternalOnly,
            Coverage::ManualReview,
            Coverage::RecommendedOnly,
            Coverage::Unsupported,
            Coverage::Missing,
            Coverage::Contradictory,
        ] {
            m.insert(c.tag(), self.count(c));
        }
        m
    }
}

// ---------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------

/// The class of a routing failure. Each class implies a different repair scope —
/// an EOL violation near a via must not rip up a whole differential pair unless
/// the local clip cannot be repaired while preserving pair symmetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DiagnosticClass {
    /// No legal terminal access.
    NoLegalAccess,
    /// A same-net component is disconnected (an open).
    DisconnectedNet,
    /// Two different nets share a resource (a short).
    DifferentNetShort,
    /// A DRC predicate violation.
    #[default]
    DrcViolation,
    /// Color / cut-mask / stitch / via non-decomposability (a native conflict).
    ColorCutMaskStitch,
    /// Antenna or EM hard violation.
    AntennaOrEm,
    /// IR-drop or current-density hard violation.
    IrOrCurrentDensity,
    /// Crosstalk/noise or coupling bound violation.
    CrosstalkNoise,
    /// A PEX bound violation.
    PexBound,
    /// An impossible analog constraint.
    ImpossibleConstraint,
    /// PDK contradiction or missing rule data.
    PdkContradiction,
    /// Budget exhausted without a proof.
    Timeout,
}

/// The repair scope a diagnosis implies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DiagnosticScope {
    /// A local, repairable violation — emit the minimal implicated region.
    #[default]
    LocalRepairable,
    /// A global infeasibility — preserve the unsatisfied core.
    GlobalInfeasible,
    /// A timeout without proof — return best verified state.
    UnprovenTimeout,
}

/// One typed routing diagnosis. For repairable cases it carries the smallest
/// implicated net region and the exact failed predicate set; for infeasible
/// cases it is part of the preserved unsatisfied core.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RouteDiagnostic {
    pub class: DiagnosticClass,
    pub scope: DiagnosticScope,
    /// A human-readable rendering.
    pub message: String,
    /// The implicated nets (the minimal rip-up seed before group closure).
    pub nets: Vec<String>,
    /// The violated predicate IDs.
    pub predicates: Vec<String>,
}

// ---------------------------------------------------------------------------
// Route quality
// ---------------------------------------------------------------------------

/// Signoff and parasitic facts reported directly (not a single scalar score).
/// The `*_rule_count` fields use the legacy coverage spelling; see
/// [`RuleCoverageReport`] for the canonical mapping.
#[derive(Debug, Clone, PartialEq)]
pub struct RouteQuality {
    pub drc_violation_count: usize,
    pub lvs_clean: bool,
    pub routed_required_net_ratio: f64,
    pub total_r_ohm: f64,
    pub total_c_ff: f64,
    pub total_coupling_ff: f64,
    pub via_count: usize,
    pub max_ir_drop_mv: f64,
    pub em_margin_min: f64,
    pub max_crosstalk_noise_mv: f64,
    pub max_length_match_delta_nm: i64,
    pub max_diff_pair_skew_nm: i64,
    pub min_area_patch_count: usize,
    pub manual_review_rule_count: usize,
    pub lvs_only_rule_count: usize,
    pub signoff_only_rule_count: usize,
    pub degraded_confidence: bool,
}

impl Default for RouteQuality {
    fn default() -> RouteQuality {
        RouteQuality {
            drc_violation_count: 0,
            lvs_clean: true,
            routed_required_net_ratio: 0.0,
            total_r_ohm: 0.0,
            total_c_ff: 0.0,
            total_coupling_ff: 0.0,
            via_count: 0,
            max_ir_drop_mv: 0.0,
            em_margin_min: f64::INFINITY,
            max_crosstalk_noise_mv: 0.0,
            max_length_match_delta_nm: 0,
            max_diff_pair_skew_nm: 0,
            min_area_patch_count: 0,
            manual_review_rule_count: 0,
            lvs_only_rule_count: 0,
            signoff_only_rule_count: 0,
            degraded_confidence: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Route certificate
// ---------------------------------------------------------------------------

/// The machine-checkable **route certificate**: geometry's companion proof. It is
/// what makes a result auditable rather than anecdotal — every hard gate carries
/// an [`Evidence`] form, and an infeasible problem preserves its unsatisfied core
/// rather than reporting a bare "routing failed".
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RouteCertificate {
    /// Hash of the route problem `(T, S, P, C, O, B)`.
    pub problem_hash: u64,
    /// Hash of the compiled PDK rule oracle.
    pub pdk_oracle_hash: u64,
    /// The classified PDK readiness state.
    pub pdk_state: PdkState,
    /// The lexicographic hard vector at acceptance time.
    pub hard: HardVector,
    /// Per-class rule coverage.
    pub coverage: RuleCoverageReport,
    /// Proof form for the DRC gate.
    pub drc_evidence: Evidence,
    /// Proof form for the LVS gate.
    pub lvs_evidence: Evidence,
    /// Whether connectivity was certified by the route-state invariant (`true`)
    /// or by the exact ILP/SMT clip engine (`false`).
    pub connectivity_by_invariant: bool,
    /// Set when a required corner/model/deck was unavailable, or access was
    /// heuristic — the result is signoff-ready *shaped*, not signoff-quality.
    pub degraded_confidence: bool,
    /// Nets routed against heuristic (non-exact) terminal access.
    pub heuristic_access_nets: Vec<String>,
    /// The preserved unsatisfied core / minimized conflict set for failures.
    pub infeasible_core: Vec<RouteDiagnostic>,
    /// The seed of any randomized net-order perturbation, for bit-reproducibility.
    pub seed: Option<u64>,
}

impl RouteCertificate {
    /// True iff the route is in the feasible region *and* not degraded — i.e. a
    /// genuine signoff-quality result.
    pub fn is_signoff_clean(&self) -> bool {
        self.hard.is_feasible() && !self.degraded_confidence
    }

    /// The honest stub certificate: feasible-shaped but explicitly
    /// degraded-confidence, with the conservative [`PdkState::SparseResearch`]
    /// state. Used by [`crate::core::run_routing`] until the engine lands.
    pub fn stub() -> RouteCertificate {
        RouteCertificate {
            pdk_state: PdkState::SparseResearch,
            degraded_confidence: true,
            ..RouteCertificate::default()
        }
    }
}

#[cfg(test)]
mod route_tests {
    use super::*;

    #[test]
    fn default_hard_vector_is_feasible() {
        assert!(HardVector::default().is_feasible());
    }

    #[test]
    fn hard_vector_lexicographic_order() {
        let clean = HardVector::default();
        let one_drc = HardVector {
            drc_violations: 1,
            ..Default::default()
        };
        let one_lvs = HardVector {
            lvs_mismatches: 1,
            ..Default::default()
        };
        // A clean route is strictly more feasible than any with a hard violation.
        assert!(clean.lex_le(&one_drc));
        assert!(!one_drc.lex_le(&clean));
        // DRC=0 (Level 1) is gated before LVS (Level 2): a route that clears DRC
        // but fails LVS ranks *better* than one that still fails DRC.
        assert!(one_lvs.lex_le(&one_drc));
        assert!(!one_drc.lex_le(&one_lvs));
    }

    #[test]
    fn stub_certificate_is_honest() {
        let cert = RouteCertificate::stub();
        // Feasible-shaped (nothing proven-wrong) but not signoff-clean.
        assert!(cert.hard.is_feasible());
        assert!(!cert.is_signoff_clean());
        assert!(cert.degraded_confidence);
        assert_eq!(cert.pdk_state, PdkState::SparseResearch);
    }

    #[test]
    fn coverage_external_only_sums_legacy_buckets() {
        let r = RuleCoverageReport {
            lvs_only: 2,
            signoff_only: 3,
            ..Default::default()
        };
        assert_eq!(r.count(Coverage::ExternalOnly), 5);
    }

    #[test]
    fn grouped_net_classes_route_jointly() {
        assert!(NetClass::DifferentialPair.is_grouped());
        assert!(!NetClass::Ordinary.is_grouped());
    }
}
