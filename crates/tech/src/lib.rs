//! PDK technology compiler for the Philis analog layout engine.
//!
//! Reads PDK descriptions (LEF, Sky130, hand-authored JSON schemas) and compiles
//! them into an immutable, queryable `CompiledTech` structure. Provides layer
//! stacks, via tables, DRC rule predicates, extraction parameters, and coverage
//! classification.
//!
//! See `crates/tech/PLAN.md` and `docs/Developer/tech/` for the full design.
