//! Via resistance model.
//!
//! See `crates/verify/PLAN.md` §5.5 for the full design.

/// Compute total via resistance for a via array.
///
/// Formula: `R = r_per_cut / num_cuts` (parallel via cuts reduce resistance).
///
/// Inputs:
///   - `num_cuts`: number of via cuts in the via array
///   - `r_per_cut`: resistance per single via cut in ohms
///
/// Output: total via resistance in ohms. Non-negative for non-negative `r_per_cut`.
///
/// # Panics
///
/// In debug builds, panics if `num_cuts == 0`.
pub fn via_resistance(num_cuts: u32, r_per_cut: f64) -> f64 {
    debug_assert!(num_cuts >= 1, "via_resistance: num_cuts must be at least 1");
    r_per_cut / num_cuts as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn via_resistance_known_values() {
        assert_eq!(via_resistance(4, 20.0), 5.0);
    }

    #[test]
    fn via_resistance_single_cut_is_identity() {
        assert_eq!(via_resistance(1, 7.5), 7.5);
    }

    #[test]
    fn via_resistance_is_non_negative() {
        assert!(via_resistance(8, 16.0) >= 0.0);
    }

    #[test]
    #[should_panic]
    fn via_resistance_panics_on_zero_cuts_in_debug() {
        via_resistance(0, 20.0);
    }
}
