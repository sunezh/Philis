//! Electromigration, IR drop, and antenna checking.
//!
//! See `crates/verify/PLAN.md` §6 for the full design.

mod antenna;
mod black;
mod ir;

pub use antenna::{check_antenna, AntennaViolation};
pub use black::{check_em, current_density, EmViolation};
pub use ir::{compute_ir_drop, IrDropResult};
