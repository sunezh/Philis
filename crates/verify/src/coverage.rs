//! Per-rule-family coverage tracking for the constraint certificate.
//!
//! See `crates/verify/PLAN.md` §3.6 for the full design.

use std::collections::HashMap;

use crate::ViolationKind;

/// All `ViolationKind` variants, used to enumerate rule families.
const ALL_KINDS: [ViolationKind; 14] = [
    ViolationKind::Overlap,
    ViolationKind::Spacing,
    ViolationKind::Width,
    ViolationKind::Enclosure,
    ViolationKind::Area,
    ViolationKind::EOL,
    ViolationKind::PRL,
    ViolationKind::CutSpacing,
    ViolationKind::SameNetNotch,
    ViolationKind::GridSnap,
    ViolationKind::OutlineExceed,
    ViolationKind::WellSpacing,
    ViolationKind::WellEnclosure,
    ViolationKind::ImplantSpacing,
];

/// Tracks how many violations of each `ViolationKind` have been checked and found.
/// Used by the constraint certificate to report per-rule-family coverage.
#[derive(Debug, Clone, Default)]
pub struct RuleFamilyCoverage {
    /// Number of candidate pairs checked per violation kind.
    pub checked: HashMap<ViolationKind, u64>,
    /// Number of violations found per violation kind.
    pub violations: HashMap<ViolationKind, u64>,
}

impl RuleFamilyCoverage {
    /// Record a check (whether or not it produced a violation).
    pub fn record_check(&mut self, kind: ViolationKind, violation: bool) {
        *self.checked.entry(kind).or_insert(0) += 1;
        if violation {
            *self.violations.entry(kind).or_insert(0) += 1;
        }
    }

    /// Total checks across all families.
    pub fn total_checked(&self) -> u64 {
        self.checked.values().sum()
    }

    /// Total violations across all families.
    pub fn total_violations(&self) -> u64 {
        self.violations.values().sum()
    }

    /// Rule families that have never been checked (potential coverage gaps).
    pub fn unchecked_families(&self) -> Vec<ViolationKind> {
        ALL_KINDS
            .iter()
            .copied()
            .filter(|kind| !self.checked.contains_key(kind))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_check_updates_checked_and_violations() {
        let mut cov = RuleFamilyCoverage::default();
        cov.record_check(ViolationKind::Spacing, false);
        cov.record_check(ViolationKind::Spacing, true);
        cov.record_check(ViolationKind::Width, true);

        assert_eq!(cov.checked[&ViolationKind::Spacing], 2);
        assert_eq!(cov.violations[&ViolationKind::Spacing], 1);
        assert_eq!(cov.checked[&ViolationKind::Width], 1);
        assert_eq!(cov.violations[&ViolationKind::Width], 1);
        assert_eq!(cov.total_checked(), 3);
        assert_eq!(cov.total_violations(), 2);
    }

    #[test]
    fn unchecked_families_excludes_checked_kinds() {
        let mut cov = RuleFamilyCoverage::default();
        cov.record_check(ViolationKind::Spacing, false);
        cov.record_check(ViolationKind::Width, false);

        let unchecked = cov.unchecked_families();
        assert_eq!(unchecked.len(), ALL_KINDS.len() - 2);
        assert!(!unchecked.contains(&ViolationKind::Spacing));
        assert!(!unchecked.contains(&ViolationKind::Width));
        assert!(unchecked.contains(&ViolationKind::Overlap));
    }

    #[test]
    fn empty_coverage_has_all_families_unchecked() {
        let cov = RuleFamilyCoverage::default();
        assert_eq!(cov.unchecked_families().len(), ALL_KINDS.len());
        assert_eq!(cov.total_checked(), 0);
        assert_eq!(cov.total_violations(), 0);
    }
}
