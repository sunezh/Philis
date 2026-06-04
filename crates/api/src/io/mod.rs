//! The foundational I/O tier — readers and writers with **zero dependencies** on
//! the engine, constraint, placement, or routing modules.
//!
//! * [`spice`] — SPICE netlist reader/writer ([`spice::SpiceNetlist`]).
//! * [`gds`] — GDSII layout reader/writer ([`gds::Gds`]); this is the
//!   gdstk-equivalent surface for manipulating geometry.
//! * [`pdk`] — PDK reader ([`pdk::Pdk`]), JSON or TOML following a small schema.
//!
//! Because this tier is self-contained, the verification checks ([`crate::verify`])
//! can read a netlist and an incrementally-built GDS at any point in the flow.

pub mod gds;
pub mod pdk;
pub mod spice;
