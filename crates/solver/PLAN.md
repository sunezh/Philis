# `philis-solver` Implementation Plan

## 1. Overview

`crates/solver` (`philis-solver`) is the shared formal-solver abstraction crate
for the Philis analog layout engine. It owns the backend-agnostic trait
interfaces for SAT, ILP, and (future) SMT solving, plus the shared algorithmic
utilities that multiple consumer crates need: MUS extraction, MCS enumeration,
and convenience constraint builders.

**Why it exists.** Two independent Philis crates need formal solvers:

- **`crates/constraints`** — SAT/SMT for MUS extraction during reconciliation,
  MCS enumeration for contradiction cores, and a lowering surface
  (`LexicographicObjective`, `BackendArtifact`) for the quality comparator.
- **`crates/route`** — SAT for pin-access compatible-subset selection (access
  oracle), ILP for via/color/min-area legality in the exact-local engine,
  SAT/SMT for grouped escape topology enumeration, and exhaustive enumeration
  for small subproblems (<20 variables).

Without a shared crate, each consumer would embed its own solver FFI bindings,
duplicate the MUS/MCS algorithms, and force users to satisfy solver dependencies
independently per crate. The `philis-solver` crate centralizes all of this
behind a single feature-flag surface, so the concrete backend (CaDiCaL, HiGHS,
Z3, custom backtracking) is a deployment decision, not an architectural one.

**Consumers:**

| Crate | Uses |
|-------|------|
| `crates/constraints` | `SatSolver` trait, `extract_mus`, `enumerate_mcs`, convenience builders |
| `crates/route` | `SatSolver` trait, `IlpSolver` trait, `ExhaustiveEnumerator`, convenience builders |
| `crates/api` | Re-exports solver status types for diagnostics in `SolveResult` |


## 2. Module Map

Target `src/` layout:

```
crates/solver/
  Cargo.toml
  PLAN.md                   # this file
  src/
    lib.rs                  # public re-exports, feature-gate wiring
    types.rs                # VarId, Lit, SolverStatus, IlpVarId, Sense, ObjDir
    sat.rs                  # SatSolver trait
    ilp.rs                  # IlpSolver trait
    mus.rs                  # extract_mus (deletion-based MUS algorithm)
    mcs.rs                  # enumerate_mcs (blocking-clause MCS enumeration)
    builders.rs             # at-most-one, exactly-one, cardinality, implication chains
    enumerate.rs            # ExhaustiveEnumerator for small instances
    backend/
      mod.rs                # backend module wiring
      exhaustive.rs         # builtin exhaustive SAT backend (always available)
      cadical.rs            # CaDiCaL SAT backend (feature = "cadical")
      highs.rs              # HiGHS ILP backend (feature = "highs")
```


## 3. `SatSolver` Trait

### Core types (`types.rs`)

```rust
/// Opaque variable identifier. Internally a `u32` index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VarId(pub u32);

/// A literal: a variable with a polarity.
/// `positive = true` means the variable itself; `false` means its negation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Lit {
    pub var: VarId,
    pub positive: bool,
}

impl Lit {
    pub fn pos(var: VarId) -> Self { Lit { var, positive: true } }
    pub fn neg(var: VarId) -> Self { Lit { var, positive: false } }
    pub fn negated(self) -> Self { Lit { var: self.var, positive: !self.positive } }
}

/// Result of a solver invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SolverStatus {
    Sat,
    Unsat,
    Unknown,
    Timeout,
}

impl SolverStatus {
    pub fn is_sat(self) -> bool { self == SolverStatus::Sat }
    pub fn is_unsat(self) -> bool { self == SolverStatus::Unsat }
}
```

### Trait definition (`sat.rs`)

```rust
use crate::types::{VarId, Lit, SolverStatus};

/// Backend-agnostic Boolean satisfiability solver.
///
/// Implementations may wrap CaDiCaL, MiniSAT, the builtin exhaustive
/// enumerator, or any DIMACS-compatible SAT solver.
pub trait SatSolver {
    /// Create a fresh Boolean variable. Returns its identifier.
    fn new_var(&mut self) -> VarId;

    /// Add a disjunctive clause over the given literals.
    ///
    /// The clause is satisfied when at least one literal is true.
    /// An empty clause is trivially unsatisfiable.
    fn add_clause(&mut self, lits: &[Lit]);

    /// Solve the current formula (all clauses added so far).
    fn solve(&mut self) -> SolverStatus;

    /// Solve under temporary assumption literals.
    ///
    /// The assumptions are not added permanently; they apply only to this
    /// call. This is the entry point for incremental solving and MUS
    /// extraction (the assumption-based paradigm).
    fn solve_under_assumptions(&mut self, assumptions: &[Lit]) -> SolverStatus;

    /// After a `Sat` result, return the variable assignments.
    ///
    /// Returns `None` if the last solve was not `Sat`. The slice is indexed
    /// by `VarId::0` — entry `i` is the assignment of `VarId(i)`.
    fn model(&self) -> Option<&[bool]>;

    /// After an `Unsat` result from `solve_under_assumptions`, return the
    /// conflict clause — the subset of assumptions that participated in the
    /// proof of unsatisfiability.
    ///
    /// Returns `None` if the last solve was not `Unsat` or if the backend
    /// does not support core extraction. The returned literals are a subset
    /// of the assumptions passed to the last `solve_under_assumptions` call.
    fn unsat_core(&self) -> Option<Vec<Lit>>;

    /// Return the number of variables created so far.
    fn num_vars(&self) -> u32;

    /// Return the number of clauses added so far.
    fn num_clauses(&self) -> u64;

    /// Set a time limit for subsequent solve calls (in milliseconds).
    /// `None` means no limit. Not all backends support this; unsupported
    /// backends ignore the call.
    fn set_timeout(&mut self, millis: Option<u64>);
}
```


## 4. `IlpSolver` Trait

### Core types (`types.rs`, additions)

```rust
/// Opaque ILP variable identifier. Internally a `u32` index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct IlpVarId(pub u32);

/// Constraint sense for an ILP row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sense {
    /// lhs <= rhs
    Le,
    /// lhs >= rhs
    Ge,
    /// lhs == rhs
    Eq,
}

/// Objective direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjDir {
    Minimize,
    Maximize,
}
```

### Trait definition (`ilp.rs`)

```rust
use crate::types::{IlpVarId, Sense, ObjDir, SolverStatus};

/// Backend-agnostic Integer/Linear Programming solver.
///
/// Implementations may wrap HiGHS, CPLEX, GLPK, or any LP/MIP backend.
pub trait IlpSolver {
    /// Create a new variable with bounds, objective coefficient, and
    /// integrality flag.
    ///
    /// - `lb` / `ub`: lower and upper bounds. Use `f64::NEG_INFINITY` /
    ///   `f64::INFINITY` for unbounded.
    /// - `obj_coeff`: coefficient of this variable in the objective function.
    /// - `is_integer`: if `true`, the variable is integer-valued (MIP).
    fn new_var(&mut self, lb: f64, ub: f64, obj_coeff: f64, is_integer: bool) -> IlpVarId;

    /// Add a linear constraint: `sum(lhs[i].1 * x[lhs[i].0]) sense rhs`.
    ///
    /// `lhs` is a list of `(variable, coefficient)` pairs.
    fn add_constraint(&mut self, lhs: &[(IlpVarId, f64)], sense: Sense, rhs: f64);

    /// Set (or replace) the objective function.
    ///
    /// `coeffs` is a list of `(variable, coefficient)` pairs. Variables not
    /// mentioned have coefficient 0.
    fn set_objective(&mut self, dir: ObjDir, coeffs: &[(IlpVarId, f64)]);

    /// Solve the current model.
    fn solve(&mut self) -> SolverStatus;

    /// After a `Sat` result, return the variable values.
    ///
    /// Returns `None` if the last solve was not `Sat`. The slice is indexed
    /// by `IlpVarId::0` — entry `i` is the value of `IlpVarId(i)`.
    fn solution(&self) -> Option<&[f64]>;

    /// After a `Sat` result, return the objective function value.
    fn objective_value(&self) -> Option<f64>;

    /// Return the number of variables created so far.
    fn num_vars(&self) -> u32;

    /// Return the number of constraints added so far.
    fn num_constraints(&self) -> u32;

    /// Set a time limit for subsequent solve calls (in milliseconds).
    /// `None` means no limit.
    fn set_timeout(&mut self, millis: Option<u64>);
}
```


## 5. MUS Extraction Utilities (`mus.rs`)

### Result type

```rust
/// Outcome of a MUS extraction attempt.
#[derive(Clone, Debug)]
pub enum MusResult {
    /// The given soft clauses contain a Minimal Unsatisfiable Subset.
    /// The `Vec<usize>` contains the indices (into the original
    /// `soft_clauses` slice) of the clauses in the MUS.
    Minimal(Vec<usize>),

    /// Budget exhausted before minimality was proven. The `Vec<usize>`
    /// contains a (possibly non-minimal) unsatisfiable subset.
    Unknown(Vec<usize>),

    /// The formula (hard clauses already in the solver + all soft clauses)
    /// is satisfiable — no MUS exists.
    Satisfiable,
}
```

### Algorithm

```rust
/// Extract a Minimal Unsatisfiable Subset from the given soft clauses.
///
/// **Preconditions:**
/// - `solver` already contains the hard clauses (added via `add_clause`).
/// - `soft_clauses` are the clauses to test for minimality. Each inner
///   `Vec<Lit>` is a disjunctive clause.
/// - `budget` is the maximum number of solver calls. `None` means unlimited.
///
/// **Algorithm:** deletion-based MUS extraction.
///
/// 1. Activate all soft clauses (via assumption literals).
/// 2. If the formula is SAT, return `Satisfiable`.
/// 3. For each soft clause `i` in the current working set:
///    a. Tentatively remove clause `i` from the working set.
///    b. Solve under assumptions for the remaining clauses.
///    c. If still UNSAT: clause `i` is redundant — keep it removed.
///    d. If SAT: clause `i` is necessary — restore it.
///    e. If budget exhausted: return `Unknown` with the current working set.
/// 4. The remaining clauses form the MUS.
///
/// The implementation uses one assumption variable per soft clause. Each
/// soft clause `C_i` is encoded as `(act_i -> C_i)`, i.e., the clause
/// `[neg(act_i)] ++ C_i` is added to the solver. Activating clause `i`
/// means assuming `act_i = true`.
pub fn extract_mus(
    solver: &mut dyn SatSolver,
    soft_clauses: &[Vec<Lit>],
    budget: Option<usize>,
) -> MusResult;
```

### Complexity

- Worst case: `O(n)` solver calls where `n = soft_clauses.len()`.
- Each solver call is a full SAT check, so total time depends on the backend.
- The budget parameter ensures the algorithm terminates predictably even on
  large instances.


## 6. MCS Enumeration (`mcs.rs`)

### API

```rust
/// Enumerate Minimal Correction Sets up to `max_count`.
///
/// An MCS is a minimal set of soft clauses whose removal makes the formula
/// satisfiable. MCS enumeration is dual to MUS extraction: every MUS is a
/// minimal hitting set of the MCS family, and vice versa.
///
/// **Preconditions:**
/// - `solver` already contains the hard clauses.
/// - `soft_clauses` are the candidate clauses.
///
/// **Algorithm:** blocking-clause based enumeration.
///
/// 1. Solve with all soft clauses activated.
/// 2. If SAT: no correction needed, return empty.
/// 3. Find one MCS (by iteratively removing clauses until SAT, then
///    checking minimality of the removed set).
/// 4. Add a blocking clause that prevents this exact MCS from recurring.
/// 5. Repeat from step 1 until `max_count` MCSes found or no more exist.
///
/// Returns a `Vec` of MCSes, where each MCS is a `Vec<usize>` of indices
/// into `soft_clauses`.
pub fn enumerate_mcs(
    solver: &mut dyn SatSolver,
    soft_clauses: &[Vec<Lit>],
    max_count: usize,
) -> Vec<Vec<usize>>;
```

### Relationship to MUS

The constraint crate's reconciliation phase uses MCS enumeration to find all
minimal ways to repair an infeasible constraint set. Each MCS corresponds to
a "repair strategy" — the smallest set of obligations that must be waived to
restore feasibility. The MUS (computed via `extract_mus`) gives the smallest
set of obligations that are jointly infeasible — the contradiction core.


## 7. Convenience Builders (`builders.rs`)

These functions add constraint patterns to an existing `SatSolver`:

### At-most-one

```rust
/// Encode "at most one of `vars` is true."
///
/// Encoding selection:
/// - `vars.len() <= 5`: pairwise encoding (n*(n-1)/2 binary clauses)
/// - `vars.len() <= 20`: sequential counter encoding (3n auxiliary variables, 4n clauses)
/// - `vars.len() > 20`: commander encoding (sqrt(n) groups, recursive)
///
/// Returns the auxiliary variables created (empty for pairwise).
pub fn at_most_one(solver: &mut dyn SatSolver, vars: &[VarId]) -> Vec<VarId>;
```

### Exactly-one

```rust
/// Encode "exactly one of `vars` is true."
///
/// Equivalent to `at_most_one(vars)` + one clause `[pos(v) for v in vars]`.
pub fn exactly_one(solver: &mut dyn SatSolver, vars: &[VarId]) -> Vec<VarId>;
```

### Cardinality

```rust
/// Encode "at most `k` of `vars` are true."
///
/// Uses a sequential counter (totalizer) encoding.
pub fn at_most_k(solver: &mut dyn SatSolver, vars: &[VarId], k: usize) -> Vec<VarId>;

/// Encode "at least `k` of `vars` are true."
///
/// Equivalent to `at_most_k(negated(vars), vars.len() - k)`.
pub fn at_least_k(solver: &mut dyn SatSolver, vars: &[VarId], k: usize) -> Vec<VarId>;

/// Encode "exactly `k` of `vars` are true."
pub fn exactly_k(solver: &mut dyn SatSolver, vars: &[VarId], k: usize) -> Vec<VarId>;
```

### Implication chains

```rust
/// Encode an implication chain: `vars[0] -> vars[1] -> ... -> vars[n-1]`.
///
/// Each implication `a -> b` is encoded as the clause `[neg(a), pos(b)]`.
pub fn implication_chain(solver: &mut dyn SatSolver, vars: &[VarId]);

/// Encode a single implication: `a -> b`.
pub fn implies(solver: &mut dyn SatSolver, a: VarId, b: VarId);
```


## 8. Backend Implementations

### `builtin` (always available) — `backend/exhaustive.rs`

```rust
/// Exhaustive SAT solver for small instances (<20 variables).
///
/// Enumerates all 2^n assignments. Intended for the route crate's small
/// subproblems (escape topology selection, local via choices) where the
/// variable count is guaranteed small and an external solver dependency
/// is not justified.
///
/// Panics or returns `Unknown` if `num_vars > 20`.
pub struct ExhaustiveSatSolver {
    vars: u32,
    clauses: Vec<Vec<Lit>>,
    model: Option<Vec<bool>>,
    timeout: Option<u64>,
}

impl SatSolver for ExhaustiveSatSolver { /* ... */ }
```

This backend is always compiled. It serves two purposes:
1. Small subproblems in the route crate where an external solver is overkill.
2. Testing: every test can run without any feature flag enabled.

### `cadical` feature — `backend/cadical.rs`

```rust
/// CaDiCaL SAT solver backend.
///
/// Wraps the `cadical` crate (Rust bindings to the CaDiCaL C++ library).
/// CaDiCaL supports incremental solving, assumption-based solving, and
/// UNSAT core extraction — all required by the MUS algorithm.
///
/// Enabled by `cargo build --features cadical`.
pub struct CadicalSolver {
    inner: cadical::Solver,
    num_vars: u32,
}

impl SatSolver for CadicalSolver { /* ... */ }
```

Cargo.toml feature:
```toml
[features]
cadical = ["dep:cadical"]

[dependencies]
cadical = { version = "2", optional = true }
```

### `highs` feature — `backend/highs.rs`

```rust
/// HiGHS ILP solver backend.
///
/// Wraps the `highs` crate (Rust bindings to the HiGHS open-source
/// LP/MIP solver). HiGHS handles LP, MIP, and QP problems.
///
/// Enabled by `cargo build --features highs`.
pub struct HighsSolver {
    inner: highs::Model,
    num_vars: u32,
    num_constraints: u32,
    solution: Option<Vec<f64>>,
    obj_value: Option<f64>,
}

impl IlpSolver for HighsSolver { /* ... */ }
```

Cargo.toml feature:
```toml
[features]
highs = ["dep:highs"]

[dependencies]
highs = { version = "1", optional = true }
```

### Future: `z3` feature

```rust
/// Z3 SMT solver backend (future).
///
/// Will implement both `SatSolver` (via Z3's propositional fragment) and
/// a future `SmtSolver` trait for theory-aware solving (bitvectors,
/// linear arithmetic, arrays). Needed for advanced reconciliation passes
/// that reason over arithmetic constraints (e.g., "this spacing + that
/// width exceeds the template bound").
```

Not in the initial build; the trait surface is designed to accommodate it.


## 9. Phase Plan (Build Order)

| Phase | What | Depends on | Gate |
|-------|------|------------|------|
| **P0** | `types.rs` — all shared types (`VarId`, `Lit`, `SolverStatus`, `IlpVarId`, `Sense`, `ObjDir`) | nothing | compiles |
| **P1** | `sat.rs` — `SatSolver` trait definition | P0 | compiles |
| **P2** | `ilp.rs` — `IlpSolver` trait definition | P0 | compiles |
| **P3** | `backend/exhaustive.rs` — builtin exhaustive SAT backend | P0, P1 | unit tests pass (SAT/UNSAT on known formulas) |
| **P4** | `builders.rs` — at-most-one, exactly-one, cardinality, implications | P0, P1, P3 | unit tests pass using exhaustive backend |
| **P5** | `mus.rs` — deletion-based MUS extraction | P0, P1, P3 | unit tests pass on known MUS instances |
| **P6** | `mcs.rs` — blocking-clause MCS enumeration | P0, P1, P3, P5 | unit tests pass on known MCS instances |
| **P7** | `enumerate.rs` — `ExhaustiveEnumerator` (all-SAT) | P0, P1, P3 | unit tests pass |
| **P8** | `backend/cadical.rs` — CaDiCaL backend | P0, P1 | feature-gated, integration tests |
| **P9** | `backend/highs.rs` — HiGHS backend | P0, P2 | feature-gated, integration tests |
| **P10** | `lib.rs` — public re-exports, feature wiring | P0-P9 | `cargo doc`, all tests green |


## 10. Public API Surface (`lib.rs`)

```rust
//! SAT/ILP/SMT solver abstraction for the Philis analog layout engine.

// ---- Core types (always available) ----
pub mod types;
pub use types::{VarId, Lit, SolverStatus, IlpVarId, Sense, ObjDir};

// ---- Trait definitions (always available) ----
pub mod sat;
pub use sat::SatSolver;

pub mod ilp;
pub use ilp::IlpSolver;

// ---- Algorithms (always available) ----
pub mod mus;
pub use mus::{extract_mus, MusResult};

pub mod mcs;
pub use mcs::enumerate_mcs;

pub mod builders;
pub use builders::{at_most_one, exactly_one, at_most_k, at_least_k, exactly_k,
                   implication_chain, implies};

pub mod enumerate;
pub use enumerate::ExhaustiveEnumerator;

// ---- Backends ----
pub mod backend;

// Always available:
pub use backend::exhaustive::ExhaustiveSatSolver;

// Feature-gated:
#[cfg(feature = "cadical")]
pub use backend::cadical::CadicalSolver;

#[cfg(feature = "highs")]
pub use backend::highs::HighsSolver;
```

### Downstream usage examples

Constraints crate (`crates/constraints/Cargo.toml`):
```toml
[dependencies]
philis-solver = { path = "../solver" }
# Optionally enable a real backend for production:
# philis-solver = { path = "../solver", features = ["cadical"] }
```

Route crate (`crates/route/Cargo.toml`):
```toml
[dependencies]
philis-solver = { path = "../solver" }
# philis-solver = { path = "../solver", features = ["cadical", "highs"] }
```


## 11. Testing Strategy

### Unit tests (no feature flags required)

All unit tests use `ExhaustiveSatSolver`, which is always available.

#### SAT/UNSAT smoke tests

```rust
#[test]
fn trivial_sat() {
    let mut s = ExhaustiveSatSolver::new();
    let a = s.new_var();
    s.add_clause(&[Lit::pos(a)]);
    assert_eq!(s.solve(), SolverStatus::Sat);
    assert_eq!(s.model().unwrap()[0], true);
}

#[test]
fn trivial_unsat() {
    let mut s = ExhaustiveSatSolver::new();
    let a = s.new_var();
    s.add_clause(&[Lit::pos(a)]);
    s.add_clause(&[Lit::neg(a)]);
    assert_eq!(s.solve(), SolverStatus::Unsat);
}

#[test]
fn assumption_based_solving() {
    let mut s = ExhaustiveSatSolver::new();
    let a = s.new_var();
    let b = s.new_var();
    s.add_clause(&[Lit::pos(a), Lit::pos(b)]);
    // Under assumption [neg(a), neg(b)], the clause is violated
    assert_eq!(
        s.solve_under_assumptions(&[Lit::neg(a), Lit::neg(b)]),
        SolverStatus::Unsat
    );
}
```

#### MUS extraction tests

```rust
#[test]
fn mus_known_instance() {
    // Hard clause: none.
    // Soft clauses:
    //   C0: [a]        — "a must be true"
    //   C1: [neg(a)]   — "a must be false"
    //   C2: [b]        — "b must be true" (irrelevant)
    //
    // The MUS is {C0, C1}. C2 is satisfiable on its own.
    let mut s = ExhaustiveSatSolver::new();
    let a = s.new_var();
    let b = s.new_var();
    let soft = vec![
        vec![Lit::pos(a)],
        vec![Lit::neg(a)],
        vec![Lit::pos(b)],
    ];
    match extract_mus(&mut s, &soft, None) {
        MusResult::Minimal(indices) => {
            assert_eq!(indices.len(), 2);
            assert!(indices.contains(&0));
            assert!(indices.contains(&1));
            assert!(!indices.contains(&2));
        }
        other => panic!("expected Minimal, got {:?}", other),
    }
}

#[test]
fn mus_satisfiable() {
    let mut s = ExhaustiveSatSolver::new();
    let a = s.new_var();
    let soft = vec![vec![Lit::pos(a)]];
    match extract_mus(&mut s, &soft, None) {
        MusResult::Satisfiable => {}
        other => panic!("expected Satisfiable, got {:?}", other),
    }
}
```

#### MCS enumeration tests

```rust
#[test]
fn mcs_enumeration() {
    // Same instance as above: MCS = {C0} or {C1}
    // (removing either one restores satisfiability)
    let mut s = ExhaustiveSatSolver::new();
    let a = s.new_var();
    let _b = s.new_var();
    let soft = vec![
        vec![Lit::pos(a)],
        vec![Lit::neg(a)],
        vec![Lit::pos(Lit::pos(VarId(1)).var)],
    ];
    let mcses = enumerate_mcs(&mut s, &soft, 10);
    assert!(mcses.len() >= 2);
    // Each MCS should be a singleton: {0} or {1}
    for mcs in &mcses {
        assert_eq!(mcs.len(), 1);
    }
}
```

#### Builder tests

```rust
#[test]
fn exactly_one_constraint() {
    let mut s = ExhaustiveSatSolver::new();
    let vars: Vec<VarId> = (0..4).map(|_| s.new_var()).collect();
    exactly_one(&mut s, &vars);
    assert_eq!(s.solve(), SolverStatus::Sat);
    let model = s.model().unwrap();
    assert_eq!(model.iter().filter(|&&v| v).count(), 1);
}
```

### Feature-gated integration tests

```rust
#[cfg(feature = "cadical")]
#[test]
fn cadical_sat_smoke() {
    let mut s = CadicalSolver::new();
    let a = s.new_var();
    s.add_clause(&[Lit::pos(a)]);
    assert_eq!(s.solve(), SolverStatus::Sat);
}

#[cfg(feature = "highs")]
#[test]
fn highs_ilp_smoke() {
    let mut s = HighsSolver::new();
    let x = s.new_var(0.0, 10.0, 1.0, true);
    s.add_constraint(&[(x, 1.0)], Sense::Le, 5.0);
    s.set_objective(ObjDir::Maximize, &[(x, 1.0)]);
    assert_eq!(s.solve(), SolverStatus::Sat);
    assert_eq!(s.solution().unwrap()[0], 5.0);
    assert_eq!(s.objective_value().unwrap(), 5.0);
}
```

### Cross-backend consistency tests

When multiple backends are enabled, a cross-check test verifies that the
exhaustive backend and CaDiCaL produce the same SAT/UNSAT results on a suite
of small formulas. This catches backend-wrapper bugs early.

```rust
#[cfg(feature = "cadical")]
#[test]
fn cross_backend_consistency() {
    for formula in SMALL_FORMULAS {
        let exhaustive_result = run_on_exhaustive(formula);
        let cadical_result = run_on_cadical(formula);
        assert_eq!(exhaustive_result, cadical_result,
            "disagreement on formula: {:?}", formula);
    }
}
```
