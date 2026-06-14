//! Connectivity extraction from layout geometry.
//!
//! See `crates/verify/PLAN.md` §4.4 for the full design.

use std::collections::HashMap;

use philis_geom::Rect;

use crate::{LayerId, ShapeRef};

use super::{ConnectivityTracker, LvsError, LvsResult, NetId};

/// Extract connectivity from a set of shapes on multiple layers.
///
/// Algorithm:
///   1. For each layer, union all shape pairs that overlap or touch.
///   2. For each via, union the shapes on the lower and upper layers that
///      the via's cut shape overlaps (via landing).
///   3. Assign net labels from the `Option<NetId>` carried by each shape.
///   4. Run `invariant_check` to detect opens and shorts.
///
/// Inputs:
///   - `shapes`: iterator of `(ShapeRef, LayerId, Rect, Option<NetId>)`.
///   - `vias`: iterator of `(ShapeRef, cut_layer, Rect)`.
///   - `via_connects`: mapping from cut layer to `(lower_layer, upper_layer)`.
///
/// Each shape becomes one element in the returned tracker, indexed in the
/// order `shapes` is iterated (element `i` corresponds to the `i`-th shape).
///
/// Output: `(ConnectivityTracker, LvsResult)`. `LvsResult::mismatches` is
/// always empty — port/device mismatches require a schematic comparison,
/// which is out of scope for geometry-only extraction.
pub fn extract_connectivity(
    shapes: impl Iterator<Item = (ShapeRef, LayerId, Rect, Option<NetId>)>,
    vias: impl Iterator<Item = (ShapeRef, LayerId, Rect)>,
    via_connects: &HashMap<LayerId, (LayerId, LayerId)>,
) -> (ConnectivityTracker, LvsResult) {
    let shapes: Vec<(ShapeRef, LayerId, Rect, Option<NetId>)> = shapes.collect();
    let mut tracker = ConnectivityTracker::new(shapes.len() as u32);

    for (i, &(_, _, _, net)) in shapes.iter().enumerate() {
        if let Some(net) = net {
            tracker.label(i as u32, net);
        }
    }

    // Same-layer shapes that overlap or touch are electrically connected.
    for i in 0..shapes.len() {
        for j in (i + 1)..shapes.len() {
            let (_, layer_i, rect_i, _) = &shapes[i];
            let (_, layer_j, rect_j, _) = &shapes[j];
            if layer_i == layer_j && rect_i.manhattan_distance(rect_j) == 0 {
                tracker.union(i as u32, j as u32);
            }
        }
    }

    // Vias union the shapes on their lower/upper layers that the cut overlaps.
    for (_via_ref, cut_layer, via_rect) in vias {
        let Some(&(lower, upper)) = via_connects.get(&cut_layer) else {
            continue;
        };
        let mut landed: Vec<u32> = Vec::new();
        for (i, &(_, layer, rect, _)) in shapes.iter().enumerate() {
            if (layer == lower || layer == upper) && rect.overlaps(&via_rect) {
                landed.push(i as u32);
            }
        }
        for &idx in landed.iter().skip(1) {
            tracker.union(landed[0], idx);
        }
    }

    let mut result = LvsResult::default();
    for err in tracker.invariant_check() {
        match err {
            LvsError::Open(open) => result.opens.push(open),
            LvsError::Short(short) => result.shorts.push(short),
        }
    }

    (tracker, result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x_lo: i64, y_lo: i64, x_hi: i64, y_hi: i64) -> Rect {
        Rect::new(x_lo, y_lo, x_hi, y_hi).unwrap()
    }

    #[test]
    fn overlapping_same_layer_shapes_form_one_net() {
        let m1 = LayerId(1);
        let shapes = vec![
            (ShapeRef(0), m1, rect(0, 0, 100, 100), Some(NetId(1))),
            (ShapeRef(1), m1, rect(50, 50, 150, 150), Some(NetId(1))),
        ];

        let (mut tracker, result) =
            extract_connectivity(shapes.into_iter(), std::iter::empty(), &HashMap::new());

        assert!(tracker.same_component(0, 1));
        assert!(result.is_clean());
    }

    #[test]
    fn touching_same_layer_shapes_are_connected() {
        let m1 = LayerId(1);
        let shapes = vec![
            (ShapeRef(0), m1, rect(0, 0, 100, 100), None),
            (ShapeRef(1), m1, rect(100, 0, 200, 100), None),
        ];

        let (mut tracker, result) =
            extract_connectivity(shapes.into_iter(), std::iter::empty(), &HashMap::new());

        assert!(tracker.same_component(0, 1));
        assert!(result.is_clean());
    }

    #[test]
    fn disconnected_same_net_shapes_report_open() {
        let m1 = LayerId(1);
        let shapes = vec![
            (ShapeRef(0), m1, rect(0, 0, 100, 100), Some(NetId(1))),
            (ShapeRef(1), m1, rect(200, 0, 300, 100), Some(NetId(1))),
        ];

        let (_tracker, result) =
            extract_connectivity(shapes.into_iter(), std::iter::empty(), &HashMap::new());

        assert_eq!(result.opens.len(), 1);
        assert_eq!(result.opens[0].net, NetId(1));
        assert!(result.shorts.is_empty());
    }

    #[test]
    fn touching_different_net_shapes_report_short() {
        let m1 = LayerId(1);
        let shapes = vec![
            (ShapeRef(0), m1, rect(0, 0, 100, 100), Some(NetId(1))),
            (ShapeRef(1), m1, rect(100, 0, 200, 100), Some(NetId(2))),
        ];

        let (_tracker, result) =
            extract_connectivity(shapes.into_iter(), std::iter::empty(), &HashMap::new());

        assert_eq!(result.shorts.len(), 1);
        assert_eq!(result.shorts[0].nets, vec![NetId(1), NetId(2)]);
        assert!(result.opens.is_empty());
    }

    #[test]
    fn via_connects_shapes_on_different_layers() {
        let m1 = LayerId(1);
        let m2 = LayerId(2);
        let via1 = LayerId(10);

        let shapes = vec![
            (ShapeRef(0), m1, rect(0, 0, 100, 100), Some(NetId(1))),
            (ShapeRef(1), m2, rect(0, 0, 100, 100), Some(NetId(1))),
        ];
        let vias = vec![(ShapeRef(2), via1, rect(40, 40, 60, 60))];
        let mut via_connects = HashMap::new();
        via_connects.insert(via1, (m1, m2));

        let (mut tracker, result) =
            extract_connectivity(shapes.into_iter(), vias.into_iter(), &via_connects);

        assert!(tracker.same_component(0, 1));
        assert!(result.is_clean());
    }

    #[test]
    fn separate_nets_remain_separate_components() {
        let m1 = LayerId(1);
        let shapes = vec![
            (ShapeRef(0), m1, rect(0, 0, 100, 100), Some(NetId(1))),
            (ShapeRef(1), m1, rect(200, 200, 300, 300), Some(NetId(2))),
        ];

        let (mut tracker, result) =
            extract_connectivity(shapes.into_iter(), std::iter::empty(), &HashMap::new());

        assert!(!tracker.same_component(0, 1));
        assert!(result.is_clean());
    }
}
