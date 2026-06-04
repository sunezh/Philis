//! DRC, LVS, and PEX verification engine for the Philis analog layout engine.
//!
//! Provides rule predicates (spacing, width, enclosure, area, EOL, PRL),
//! violation types, incremental DRC checking infrastructure, connectivity
//! extraction (LVS via union-find), and parasitic extraction surrogates (PEX).
//!
//! See `crates/verify/PLAN.md` and `docs/Developer/verify/` for the full design.
