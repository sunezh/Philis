//! Device ports and the low-coupling [`PortRef`] the builder accepts.
//!
//! **Design rule — accept the caller's representation.** [`crate::NetBuilder::connect`]
//! takes `impl Into<PortRef>`, so a caller may pass a *typed* port
//! (`MosfetPort::Gate`) for compile-time safety, or look one up *by string*
//! ([`port_ref_from_name`]) when the port name comes from data. Neither
//! vocabulary is forced.

/// A MOSFET terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MosfetPort {
    Gate,
    Drain,
    Source,
    Bulk,
}

/// A two-terminal passive (resistor / capacitor / diode) terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassivePort {
    Plus,
    Minus,
}

/// A bipolar-junction-transistor terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BjtPort {
    Collector,
    Base,
    Emitter,
}

impl MosfetPort {
    /// The terminal's positional index.
    pub const fn idx(self) -> u8 {
        match self {
            MosfetPort::Gate => 0,
            MosfetPort::Drain => 1,
            MosfetPort::Source => 2,
            MosfetPort::Bulk => 3,
        }
    }
    /// The canonical terminal name.
    pub const fn name(self) -> &'static str {
        match self {
            MosfetPort::Gate => "gate",
            MosfetPort::Drain => "drain",
            MosfetPort::Source => "source",
            MosfetPort::Bulk => "bulk",
        }
    }
}

impl PassivePort {
    pub const fn idx(self) -> u8 {
        match self {
            PassivePort::Plus => 0,
            PassivePort::Minus => 1,
        }
    }
    pub const fn name(self) -> &'static str {
        match self {
            PassivePort::Plus => "plus",
            PassivePort::Minus => "minus",
        }
    }
}

impl BjtPort {
    pub const fn idx(self) -> u8 {
        match self {
            BjtPort::Collector => 0,
            BjtPort::Base => 1,
            BjtPort::Emitter => 2,
        }
    }
    pub const fn name(self) -> &'static str {
        match self {
            BjtPort::Collector => "collector",
            BjtPort::Base => "base",
            BjtPort::Emitter => "emitter",
        }
    }
}

/// An opaque reference to one device terminal: its index and canonical name.
///
/// This is the single type [`crate::NetBuilder::connect`] accepts. It is built
/// `From` any of the typed port enums, or by string lookup, so the surface has
/// exactly one connection vocabulary while staying open to several inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortRef {
    idx: u8,
    name: &'static str,
}

impl PortRef {
    /// The terminal's positional index.
    pub const fn idx(&self) -> u8 {
        self.idx
    }
    /// The terminal's canonical name.
    pub const fn name(&self) -> &'static str {
        self.name
    }
}

impl From<MosfetPort> for PortRef {
    fn from(p: MosfetPort) -> PortRef {
        PortRef {
            idx: p.idx(),
            name: p.name(),
        }
    }
}

impl From<PassivePort> for PortRef {
    fn from(p: PassivePort) -> PortRef {
        PortRef {
            idx: p.idx(),
            name: p.name(),
        }
    }
}

impl From<BjtPort> for PortRef {
    fn from(p: BjtPort) -> PortRef {
        PortRef {
            idx: p.idx(),
            name: p.name(),
        }
    }
}

/// The kind of a primitive device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Nmos,
    Pmos,
    Resistor,
    Capacitor,
    Bjt,
    Diode,
    Generic,
}

/// Look a [`PortRef`] up by device kind and terminal name (case-insensitive).
///
/// Accepts `"body"` as an alias for a MOSFET `Bulk`. Returns `None` for an
/// unknown name. This is the string-keyed sibling of the typed `From` impls.
pub fn port_ref_from_name(kind: DeviceKind, name: &str) -> Option<PortRef> {
    let n = name.to_ascii_lowercase();
    let mosfet = |p: MosfetPort| Some(PortRef::from(p));
    let passive = |p: PassivePort| Some(PortRef::from(p));
    let bjt = |p: BjtPort| Some(PortRef::from(p));
    match kind {
        DeviceKind::Nmos | DeviceKind::Pmos | DeviceKind::Generic => match n.as_str() {
            "gate" | "g" => mosfet(MosfetPort::Gate),
            "drain" | "d" => mosfet(MosfetPort::Drain),
            "source" | "s" => mosfet(MosfetPort::Source),
            "bulk" | "body" | "b" => mosfet(MosfetPort::Bulk),
            _ => None,
        },
        DeviceKind::Resistor | DeviceKind::Capacitor | DeviceKind::Diode => match n.as_str() {
            "plus" | "p" | "+" | "anode" => passive(PassivePort::Plus),
            "minus" | "m" | "-" | "cathode" => passive(PassivePort::Minus),
            _ => None,
        },
        DeviceKind::Bjt => match n.as_str() {
            "collector" | "c" => bjt(BjtPort::Collector),
            "base" | "b" => bjt(BjtPort::Base),
            "emitter" | "e" => bjt(BjtPort::Emitter),
            _ => None,
        },
    }
}

/// The legal canonical port names for a device kind.
pub fn valid_port_names(kind: DeviceKind) -> &'static [&'static str] {
    match kind {
        DeviceKind::Nmos | DeviceKind::Pmos | DeviceKind::Generic => {
            &["gate", "drain", "source", "bulk"][..]
        }
        DeviceKind::Resistor | DeviceKind::Capacitor | DeviceKind::Diode => &["plus", "minus"][..],
        DeviceKind::Bjt => &["collector", "base", "emitter"][..],
    }
}
