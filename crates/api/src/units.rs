//! Value types: lengths, integer-nanometer coordinates, and the coordinate axis.
//!
//! Two length representations, on purpose:
//! * [`Length`] — floating-point **meters**, the natural unit for a caller typing
//!   device geometry (`Length::um(2.0)`).
//! * [`Distance`] / [`Coord`] — integer **nanometers**, the exact unit of the
//!   placement grid.
//!
//! All three round-trip through nanometers and share the same `um`/`nm`
//! constructor vocabulary, so the caller learns one unit idiom, not three.

/// An SI length in meters (floating point).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Length(f64);

impl Length {
    /// From meters.
    pub const fn m(v: f64) -> Length {
        Length(v)
    }
    /// From micrometers.
    pub const fn um(v: f64) -> Length {
        Length(v * 1e-6)
    }
    /// From nanometers.
    pub const fn nm(v: f64) -> Length {
        Length(v * 1e-9)
    }
    /// From picometers.
    pub const fn pm(v: f64) -> Length {
        Length(v * 1e-12)
    }
    /// Raw meters out.
    pub const fn as_meters(self) -> f64 {
        self.0
    }
    /// Rounded integer nanometers.
    pub fn to_nm_i64(self) -> i64 {
        (self.0 * 1e9).round() as i64
    }
}

/// An integer-nanometer spacing or offset (relative).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Distance(pub i64);

impl Distance {
    /// From raw nanometers.
    pub const fn new(nm: i64) -> Distance {
        Distance(nm)
    }
    /// From nanometers (alias of [`Distance::new`], for vocabulary symmetry).
    pub const fn nm(nm: i64) -> Distance {
        Distance(nm)
    }
    /// From micrometers.
    pub const fn um(um: i64) -> Distance {
        Distance(um * 1_000)
    }
    /// Raw nanometers out.
    pub const fn as_nm(self) -> i64 {
        self.0
    }
}

/// An integer-nanometer absolute coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Coord(pub i64);

impl Coord {
    /// From raw nanometers.
    pub const fn new(nm: i64) -> Coord {
        Coord(nm)
    }
    /// From nanometers (alias of [`Coord::new`]).
    pub const fn nm(nm: i64) -> Coord {
        Coord(nm)
    }
    /// From micrometers.
    pub const fn um(um: i64) -> Coord {
        Coord(um * 1_000)
    }
    /// Raw nanometers out.
    pub const fn as_nm(self) -> i64 {
        self.0
    }
}

impl From<Length> for Distance {
    fn from(l: Length) -> Distance {
        Distance(l.to_nm_i64())
    }
}

impl From<Length> for Coord {
    fn from(l: Length) -> Coord {
        Coord(l.to_nm_i64())
    }
}

/// A coordinate axis: horizontal (`X`) or vertical (`Y`).
///
/// This is the façade-owned axis type — building an ordering or alignment
/// constraint never requires importing an engine-internal datatype.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    /// The horizontal axis.
    X,
    /// The vertical axis.
    Y,
}

impl Axis {
    /// The canonical lowercase name (`"x"` / `"y"`).
    pub const fn name(self) -> &'static str {
        match self {
            Axis::X => "x",
            Axis::Y => "y",
        }
    }
}

/// Parse an axis name. Returns the façade-owned [`Axis`].
pub fn parse_axis(s: &str) -> Result<Axis, &'static str> {
    match s.to_ascii_lowercase().as_str() {
        "horizontal" | "x" => Ok(Axis::X),
        "vertical" | "y" => Ok(Axis::Y),
        _ => Err("axis must be one of: horizontal, x, vertical, y"),
    }
}
