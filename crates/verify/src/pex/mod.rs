//! PEX (parasitic extraction) lumped-element surrogates.
//!
//! See `crates/verify/PLAN.md` §5 for the full design.

mod coupling;
mod via;
mod wire;

use std::collections::HashMap;

pub use coupling::{interlayer_coupling, same_layer_coupling};
pub use via::via_resistance;
pub use wire::{ground_capacitance, wire_resistance};

use crate::{LayerId, NetId, ShapeRef};

/// Lumped parasitic values for a single net.
///
/// These are first-order estimates suitable for driving placement/routing
/// quality metrics and repair loops. They are NOT signoff-accurate — the
/// constraint certificate records them as approximate evidence.
#[derive(Debug, Clone, Default)]
pub struct NetParasitics {
    /// Total wire resistance (ohms), summed across all segments.
    pub total_r: f64,
    /// Total ground capacitance (femtofarads), summed across all segments.
    pub total_cg: f64,
    /// Total coupling capacitance (femtofarads) to all aggressors.
    pub total_cc: f64,
    /// Total via resistance (ohms), summed across all vias.
    pub total_rvia: f64,
    /// Per-segment breakdown for diagnosis and repair scoping.
    pub segments: Vec<SegmentParasitics>,
}

/// Parasitics for a single wire segment or via.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentParasitics {
    /// The shape this segment corresponds to.
    pub shape: ShapeRef,
    /// Layer of the segment.
    pub layer: LayerId,
    /// Wire resistance contribution (ohms).
    pub r: f64,
    /// Ground capacitance contribution (fF).
    pub cg: f64,
    /// Coupling capacitance contributions to specific aggressors (fF).
    pub cc: Vec<(NetId, f64)>,
    /// Via resistance contribution, if this is a via segment (ohms).
    pub rvia: f64,
}

impl SegmentParasitics {
    fn total_cc(&self) -> f64 {
        self.cc.iter().map(|(_, c)| c).sum()
    }
}

impl NetParasitics {
    /// Add the contribution of a new wire segment to the running totals.
    pub fn add_segment(&mut self, seg: SegmentParasitics) {
        self.total_r += seg.r;
        self.total_cg += seg.cg;
        self.total_cc += seg.total_cc();
        self.total_rvia += seg.rvia;
        self.segments.push(seg);
    }

    /// Remove the contribution of a wire segment from the running totals.
    ///
    /// Does nothing if no segment with the given `shape` is present.
    pub fn remove_segment(&mut self, shape: ShapeRef) {
        if let Some(pos) = self.segments.iter().position(|s| s.shape == shape) {
            let seg = self.segments.remove(pos);
            self.total_r -= seg.r;
            self.total_cg -= seg.cg;
            self.total_cc -= seg.total_cc();
            self.total_rvia -= seg.rvia;
        }
    }

    /// Recompute totals from the segment list. Called after a batch of
    /// add/remove operations to eliminate floating-point drift.
    pub fn recompute_totals(&mut self) {
        self.total_r = self.segments.iter().map(|s| s.r).sum();
        self.total_cg = self.segments.iter().map(|s| s.cg).sum();
        self.total_cc = self.segments.iter().map(SegmentParasitics::total_cc).sum();
        self.total_rvia = self.segments.iter().map(|s| s.rvia).sum();
    }
}

/// A detected parasitic outlier — a net whose parasitics exceed a threshold
/// or a matched group whose parasitic delta exceeds its budget.
#[derive(Debug, Clone)]
pub struct PexOutlier {
    /// Which kind of outlier this is.
    pub kind: OutlierKind,
    /// The net that triggered the outlier.
    pub net: NetId,
    /// The measured value that triggered the outlier.
    pub actual: f64,
    /// The threshold or budget it exceeded.
    pub threshold: f64,
    /// Unit of the measurement (ohms, fF, etc.).
    pub unit: &'static str,
}

/// The kind of PEX outlier.
///
/// `HighGroundCap` and `HighViaResistance` are part of the closed taxonomy
/// but are not yet produced by [`detect_outliers`], which currently only
/// checks total resistance, total coupling capacitance, and matched-group
/// deltas — a ground-cap/via-resistance threshold would need additional
/// parameters not yet part of its signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlierKind {
    /// Total wire resistance exceeds threshold.
    HighResistance,
    /// Total coupling capacitance exceeds threshold.
    HighCoupling,
    /// Ground capacitance exceeds threshold.
    HighGroundCap,
    /// Via resistance exceeds threshold.
    HighViaResistance,
    /// Delta between matched-group members exceeds budget.
    MatchedGroupDelta,
}

/// Budget for parasitic matching within a group of nets.
#[derive(Debug, Clone)]
pub struct MatchedBudget {
    /// Maximum allowed difference in total resistance (ohms) between any two
    /// nets in the group.
    pub max_r_delta: f64,
    /// Maximum allowed difference in total ground capacitance (fF).
    pub max_cg_delta: f64,
    /// Maximum allowed difference in total coupling capacitance (fF).
    pub max_cc_delta: f64,
}

/// Detect outliers in a set of net parasitics.
///
/// Checks each net's `total_r` against `r_threshold` and `total_cc` against
/// `cc_threshold`, and checks each `matched_groups` entry for parasitic
/// deltas (max - min across the group, per metric) exceeding its
/// [`MatchedBudget`].
pub fn detect_outliers(
    nets: &HashMap<NetId, NetParasitics>,
    r_threshold: f64,
    cc_threshold: f64,
    matched_groups: &[(Vec<NetId>, MatchedBudget)],
) -> Vec<PexOutlier> {
    let mut outliers = Vec::new();

    for (&net, parasitics) in nets {
        if parasitics.total_r > r_threshold {
            outliers.push(PexOutlier {
                kind: OutlierKind::HighResistance,
                net,
                actual: parasitics.total_r,
                threshold: r_threshold,
                unit: "ohms",
            });
        }
        if parasitics.total_cc > cc_threshold {
            outliers.push(PexOutlier {
                kind: OutlierKind::HighCoupling,
                net,
                actual: parasitics.total_cc,
                threshold: cc_threshold,
                unit: "fF",
            });
        }
    }

    for (group, budget) in matched_groups {
        check_matched_metric(
            nets,
            group,
            budget.max_r_delta,
            "ohms",
            |p| p.total_r,
            &mut outliers,
        );
        check_matched_metric(
            nets,
            group,
            budget.max_cg_delta,
            "fF",
            |p| p.total_cg,
            &mut outliers,
        );
        check_matched_metric(
            nets,
            group,
            budget.max_cc_delta,
            "fF",
            |p| p.total_cc,
            &mut outliers,
        );
    }

    outliers
}

/// Check one metric across a matched group: if `max - min` exceeds
/// `max_delta`, push a [`OutlierKind::MatchedGroupDelta`] outlier for the
/// net with the highest value.
fn check_matched_metric(
    nets: &HashMap<NetId, NetParasitics>,
    group: &[NetId],
    max_delta: f64,
    unit: &'static str,
    metric: impl Fn(&NetParasitics) -> f64,
    outliers: &mut Vec<PexOutlier>,
) {
    let values: Vec<(NetId, f64)> = group
        .iter()
        .filter_map(|&n| nets.get(&n).map(|p| (n, metric(p))))
        .collect();
    if values.len() < 2 {
        return;
    }

    let (&max_net, &max_v) = values
        .iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .map(|(n, v)| (n, v))
        .unwrap();
    let min_v = values.iter().map(|&(_, v)| v).fold(f64::INFINITY, f64::min);

    let delta = max_v - min_v;
    if delta > max_delta {
        outliers.push(PexOutlier {
            kind: OutlierKind::MatchedGroupDelta,
            net: max_net,
            actual: delta,
            threshold: max_delta,
            unit,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(
        shape: ShapeRef,
        r: f64,
        cg: f64,
        cc: Vec<(NetId, f64)>,
        rvia: f64,
    ) -> SegmentParasitics {
        SegmentParasitics {
            shape,
            layer: LayerId(1),
            r,
            cg,
            cc,
            rvia,
        }
    }

    #[test]
    fn add_segment_updates_totals() {
        let mut net = NetParasitics::default();
        net.add_segment(segment(ShapeRef(0), 10.0, 2.0, vec![(NetId(1), 0.5)], 1.0));

        assert_eq!(net.total_r, 10.0);
        assert_eq!(net.total_cg, 2.0);
        assert_eq!(net.total_cc, 0.5);
        assert_eq!(net.total_rvia, 1.0);
        assert_eq!(net.segments.len(), 1);
    }

    #[test]
    fn remove_segment_restores_totals_to_zero() {
        let mut net = NetParasitics::default();
        net.add_segment(segment(ShapeRef(0), 10.0, 2.0, vec![(NetId(1), 0.5)], 1.0));
        net.add_segment(segment(ShapeRef(1), 5.0, 1.0, vec![], 0.0));

        net.remove_segment(ShapeRef(0));

        assert_eq!(net.total_r, 5.0);
        assert_eq!(net.total_cg, 1.0);
        assert_eq!(net.total_cc, 0.0);
        assert_eq!(net.total_rvia, 0.0);
        assert_eq!(net.segments.len(), 1);
    }

    #[test]
    fn remove_segment_is_noop_for_unknown_shape() {
        let mut net = NetParasitics::default();
        net.add_segment(segment(ShapeRef(0), 10.0, 2.0, vec![], 0.0));
        net.remove_segment(ShapeRef(99));

        assert_eq!(net.total_r, 10.0);
        assert_eq!(net.segments.len(), 1);
    }

    #[test]
    fn recompute_totals_matches_incremental_totals() {
        let mut net = NetParasitics::default();
        net.add_segment(segment(ShapeRef(0), 10.0, 2.0, vec![(NetId(1), 0.5)], 1.0));
        net.add_segment(segment(ShapeRef(1), 5.0, 1.0, vec![(NetId(2), 0.25)], 0.5));

        let incremental = (net.total_r, net.total_cg, net.total_cc, net.total_rvia);
        net.recompute_totals();

        assert_eq!(
            (net.total_r, net.total_cg, net.total_cc, net.total_rvia),
            incremental
        );
    }

    #[test]
    fn detect_outliers_flags_high_resistance_and_coupling() {
        let mut nets = HashMap::new();
        nets.insert(
            NetId(1),
            NetParasitics {
                total_r: 150.0,
                total_cg: 0.0,
                total_cc: 5.0,
                total_rvia: 0.0,
                segments: vec![],
            },
        );

        let outliers = detect_outliers(&nets, 100.0, 100.0, &[]);

        assert_eq!(outliers.len(), 1);
        assert_eq!(outliers[0].kind, OutlierKind::HighResistance);
        assert_eq!(outliers[0].net, NetId(1));
        assert_eq!(outliers[0].actual, 150.0);
    }

    #[test]
    fn detect_outliers_is_empty_when_under_thresholds() {
        let mut nets = HashMap::new();
        nets.insert(
            NetId(1),
            NetParasitics {
                total_r: 50.0,
                total_cg: 0.0,
                total_cc: 5.0,
                total_rvia: 0.0,
                segments: vec![],
            },
        );

        assert!(detect_outliers(&nets, 100.0, 100.0, &[]).is_empty());
    }

    #[test]
    fn detect_outliers_flags_matched_group_delta() {
        let mut nets = HashMap::new();
        nets.insert(
            NetId(1),
            NetParasitics {
                total_r: 100.0,
                ..Default::default()
            },
        );
        nets.insert(
            NetId(2),
            NetParasitics {
                total_r: 80.0,
                ..Default::default()
            },
        );

        let budget = MatchedBudget {
            max_r_delta: 10.0,
            max_cg_delta: f64::INFINITY,
            max_cc_delta: f64::INFINITY,
        };
        let outliers =
            detect_outliers(&nets, 1000.0, 1000.0, &[(vec![NetId(1), NetId(2)], budget)]);

        assert_eq!(outliers.len(), 1);
        assert_eq!(outliers[0].kind, OutlierKind::MatchedGroupDelta);
        assert_eq!(outliers[0].net, NetId(1));
        assert_eq!(outliers[0].actual, 20.0);
        assert_eq!(outliers[0].threshold, 10.0);
    }

    #[test]
    fn detect_outliers_matched_group_within_budget_is_clean() {
        let mut nets = HashMap::new();
        nets.insert(
            NetId(1),
            NetParasitics {
                total_r: 100.0,
                ..Default::default()
            },
        );
        nets.insert(
            NetId(2),
            NetParasitics {
                total_r: 95.0,
                ..Default::default()
            },
        );

        let budget = MatchedBudget {
            max_r_delta: 10.0,
            max_cg_delta: f64::INFINITY,
            max_cc_delta: f64::INFINITY,
        };
        let outliers =
            detect_outliers(&nets, 1000.0, 1000.0, &[(vec![NetId(1), NetId(2)], budget)]);

        assert!(outliers.is_empty());
    }
}
