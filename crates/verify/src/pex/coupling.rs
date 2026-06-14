//! Coupling capacitance models.
//!
//! See `crates/verify/PLAN.md` §5.4 for the full design.

use super::NM_PER_UM;

/// Square nanometers per square micron, used to convert nm^2 inputs to um^2
/// for fF-scale results.
const NM2_PER_UM2: f64 = NM_PER_UM * NM_PER_UM;

/// Compute same-layer coupling capacitance between two parallel wire segments.
///
/// Formula: `Cc = prl_um * coupling_fn(spacing)`
///
/// Inputs:
///   - `prl`: parallel run length in nm
///   - `spacing`: edge-to-edge spacing in nm
///   - `coupling_fn`: returns coupling capacitance in fF/um for the given
///     spacing in nm; expected to be monotonically decreasing with spacing
///
/// Output: coupling capacitance in fF. Non-negative for non-negative inputs
/// and a non-negative `coupling_fn`.
pub fn same_layer_coupling(prl: i64, spacing: i64, coupling_fn: impl Fn(i64) -> f64) -> f64 {
    let prl_um = prl as f64 / NM_PER_UM;
    prl_um * coupling_fn(spacing)
}

/// Compute interlayer (adjacent-layer) coupling capacitance.
///
/// Formula: `Cc = overlap_area_um2 * interlayer_cap`
///
/// Inputs:
///   - `overlap_area`: area of intersection of the two shapes' projections
///     onto the XY plane, in nm^2
///   - `interlayer_cap`: parallel-plate capacitance per unit area in fF/um^2
///
/// Output: coupling capacitance in fF. Non-negative for non-negative inputs.
pub fn interlayer_coupling(overlap_area: i64, interlayer_cap: f64) -> f64 {
    let overlap_area_um2 = overlap_area as f64 / NM2_PER_UM2;
    overlap_area_um2 * interlayer_cap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_layer_coupling_known_value() {
        // 2um prl, coupling_fn returns a constant 1.5 fF/um -> 2 * 1.5 = 3.0
        assert_eq!(same_layer_coupling(2000, 100, |_| 1.5), 3.0);
    }

    #[test]
    fn same_layer_coupling_zero_prl_is_zero() {
        assert_eq!(same_layer_coupling(0, 100, |_| 1.5), 0.0);
    }

    #[test]
    fn same_layer_coupling_passes_spacing_to_fn() {
        let cc = same_layer_coupling(1000, 200, |spacing| {
            assert_eq!(spacing, 200);
            1.0
        });
        assert_eq!(cc, 1.0);
    }

    #[test]
    fn same_layer_coupling_is_non_negative() {
        assert!(same_layer_coupling(5000, 50, |s| 1.0 / s as f64) >= 0.0);
    }

    #[test]
    fn interlayer_coupling_known_value() {
        // 1um^2 (1_000_000 nm^2), interlayer_cap=0.5 fF/um^2 -> 0.5 fF
        assert_eq!(interlayer_coupling(1_000_000, 0.5), 0.5);
    }

    #[test]
    fn interlayer_coupling_zero_area_is_zero() {
        assert_eq!(interlayer_coupling(0, 0.5), 0.0);
    }

    #[test]
    fn interlayer_coupling_is_non_negative() {
        assert!(interlayer_coupling(250_000, 0.8) >= 0.0);
    }
}
