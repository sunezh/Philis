//! Antenna ratio checking.
//!
//! See `crates/verify/PLAN.md` §6.3 for the full design.

use crate::{LayerId, NetId};

/// Antenna ratio check for a single net at a single layer.
///
/// Formula: `ratio = metal_area / gate_oxide_area`
///
/// If `ratio > max_ratio` for the layer, this is an antenna violation. The
/// check is cumulative: `metal_area` is expected to include all metal on the
/// checked layer and below (or above, depending on the process direction).
///
/// Inputs:
///   - `metal_area`: total metal area connected to the gate on this layer (nm^2)
///   - `gate_oxide_area`: total gate oxide area connected to this net (nm^2)
///   - `max_ratio`: maximum allowed antenna ratio for the layer (from tech)
///   - `net`: the net being checked
///   - `layer`: the metal layer being checked
///
/// Output: `Some(AntennaViolation)` if `ratio > max_ratio` and
/// `gate_oxide_area > 0`, otherwise `None`. A net with no gate oxide
/// (`gate_oxide_area == 0`) has no antenna exposure and never violates.
pub fn check_antenna(
    metal_area: i64,
    gate_oxide_area: i64,
    max_ratio: f64,
    net: NetId,
    layer: LayerId,
) -> Option<AntennaViolation> {
    if gate_oxide_area <= 0 {
        return None;
    }
    let ratio = metal_area as f64 / gate_oxide_area as f64;
    if ratio > max_ratio {
        Some(AntennaViolation {
            net,
            layer,
            ratio,
            max_ratio,
            metal_area,
            gate_oxide_area,
        })
    } else {
        None
    }
}

/// An antenna violation: a net's accumulated metal area on a layer exceeds
/// the allowed ratio relative to its connected gate oxide area.
#[derive(Debug, Clone)]
pub struct AntennaViolation {
    /// The net being checked.
    pub net: NetId,
    /// The metal layer being checked.
    pub layer: LayerId,
    /// Computed antenna ratio (`metal_area / gate_oxide_area`).
    pub ratio: f64,
    /// Maximum allowed ratio.
    pub max_ratio: f64,
    /// Metal area (nm^2).
    pub metal_area: i64,
    /// Gate oxide area (nm^2).
    pub gate_oxide_area: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_when_ratio_exceeds_max() {
        let v = check_antenna(1000, 10, 50.0, NetId(1), LayerId(1)).unwrap();
        assert_eq!(v.ratio, 100.0);
        assert_eq!(v.max_ratio, 50.0);
        assert_eq!(v.metal_area, 1000);
        assert_eq!(v.gate_oxide_area, 10);
        assert_eq!(v.net, NetId(1));
        assert_eq!(v.layer, LayerId(1));
    }

    #[test]
    fn does_not_fire_when_under_max_ratio() {
        assert!(check_antenna(100, 10, 50.0, NetId(1), LayerId(1)).is_none());
    }

    #[test]
    fn does_not_fire_at_exactly_max_ratio() {
        assert!(check_antenna(500, 10, 50.0, NetId(1), LayerId(1)).is_none());
    }

    #[test]
    fn does_not_fire_when_gate_oxide_area_is_zero() {
        assert!(check_antenna(1_000_000, 0, 50.0, NetId(1), LayerId(1)).is_none());
    }

    #[test]
    fn does_not_fire_when_gate_oxide_area_is_negative() {
        assert!(check_antenna(1_000_000, -10, 50.0, NetId(1), LayerId(1)).is_none());
    }

    #[test]
    fn zero_metal_area_is_zero_ratio() {
        assert!(check_antenna(0, 10, 50.0, NetId(1), LayerId(1)).is_none());
    }
}
