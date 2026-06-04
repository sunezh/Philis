//! SAT/ILP/SMT solver abstraction for the Philis analog layout engine.
//!
//! Provides backend-agnostic traits (`SatSolver`, `IlpSolver`) and shared
//! utilities (MUS/MCS extraction, solver status types). Concrete backends
//! are gated behind cargo features.
//!
//! See `crates/solver/PLAN.md` and `docs/Developer/solver/` for the full design.
