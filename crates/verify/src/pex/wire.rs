//! Wire resistance and ground capacitance models.
//!
//! See `crates/verify/PLAN.md` §5.2-5.3 for the full design.

use super::NM_PER_UM;

/// Compute wire resistance for a rectangular segment.
///
/// Formula: `R = (length / width) * sheet_r`
///
/// Inputs:
///   - `length`: wire length in nm, along the current flow direction
///   - `width`: wire width in nm, perpendicular to the current flow
///   - `sheet_r`: sheet resistance in ohms/square
///
/// Output: resistance in ohms.
///
/// # Panics
///
/// In debug builds, panics if `width <= 0`.
pub fn wire_resistance(length: i64, width: i64, sheet_r: f64) -> f64 {
    debug_assert!(width > 0, "wire_resistance: width must be positive");
    (length as f64 / width as f64) * sheet_r
}

/// Compute ground (substrate) capacitance for a rectangular segment.
///
/// Formula: `Cg = length_um * width_um * area_cap + 2 * length_um * fringe_cap`
///
/// Inputs:
///   - `length`: wire length in nm
///   - `width`: wire width in nm
///   - `area_cap`: areal capacitance in fF/um^2
///   - `fringe_cap`: fringe capacitance per unit length in fF/um
///
/// Output: ground capacitance in fF. Non-negative for non-negative inputs.
pub fn ground_capacitance(length: i64, width: i64, area_cap: f64, fringe_cap: f64) -> f64 {
    let length_um = length as f64 / NM_PER_UM;
    let width_um = width as f64 / NM_PER_UM;
    length_um * width_um * area_cap + 2.0 * length_um * fringe_cap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_resistance_known_geometry() {
        // 1000nm long, 500nm wide, 10 ohms/sq -> (1000/500) * 10 = 20 ohms.
        assert_eq!(wire_resistance(1000, 500, 10.0), 20.0);
    }

    #[test]
    fn wire_resistance_zero_length_is_zero() {
        assert_eq!(wire_resistance(0, 500, 10.0), 0.0);
    }

    #[test]
    fn wire_resistance_is_non_negative() {
        assert!(wire_resistance(2000, 100, 5.0) >= 0.0);
    }

    #[test]
    #[should_panic]
    fn wire_resistance_panics_on_zero_width_in_debug() {
        wire_resistance(1000, 0, 10.0);
    }

    #[test]
    fn ground_capacitance_known_geometry() {
        // 1um x 1um, area_cap=2.0 fF/um^2, fringe_cap=0.5 fF/um
        // Cg = 1*1*2.0 + 2*1*0.5 = 3.0
        assert_eq!(ground_capacitance(1000, 1000, 2.0, 0.5), 3.0);
    }

    #[test]
    fn ground_capacitance_zero_length_is_zero() {
        assert_eq!(ground_capacitance(0, 1000, 2.0, 0.5), 0.0);
    }

    #[test]
    fn ground_capacitance_is_non_negative() {
        assert!(ground_capacitance(3000, 200, 1.5, 0.2) >= 0.0);
    }
}
