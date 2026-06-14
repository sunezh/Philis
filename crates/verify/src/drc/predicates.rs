//! Pure DRC rule predicates.
//!
//! Each predicate is a stateless function over geometry and rule parameters.
//! Callers own broad-phase candidate generation (spatial indexing); these
//! predicates only decide whether a given candidate pair (or single shape)
//! violates a rule.
//!
//! See `crates/verify/PLAN.md` §3.3 for the full design.

use philis_geom::Rect;

use super::{DrcViolation, LayerId, RuleId, ShapeRef, ViolationKind};

/// The bounding box of `a` and `b` together, used as a violation's `region`.
fn bbox(a: &Rect, b: &Rect) -> Rect {
    Rect::new(
        a.x_lo.min(b.x_lo),
        a.y_lo.min(b.y_lo),
        a.x_hi.max(b.x_hi),
        a.y_hi.max(b.y_hi),
    )
    .expect("union of two valid rects is a valid rect")
}

/// Shared spacing-style check: fires when `a` and `b` do not overlap and
/// their [`Rect::manhattan_distance`] is less than `min_spacing`. Used by
/// [`check_spacing`], [`check_cut_spacing`], [`check_well_spacing`], and
/// [`check_implant_spacing`], which differ only in [`ViolationKind`] and
/// whether `same_net` is meaningful.
#[allow(clippy::too_many_arguments)]
fn check_spacing_like(
    a: &Rect,
    b: &Rect,
    a_ref: ShapeRef,
    b_ref: ShapeRef,
    layer: LayerId,
    rule: RuleId,
    min_spacing: i64,
    same_net: bool,
    kind: ViolationKind,
) -> Option<DrcViolation> {
    if a.overlaps(b) {
        return None;
    }
    let actual = a.manhattan_distance(b);
    if actual < min_spacing {
        Some(DrcViolation {
            kind,
            rule,
            layer,
            shapes: (a_ref, Some(b_ref)),
            region: bbox(a, b),
            required: min_spacing,
            actual,
            same_net,
        })
    } else {
        None
    }
}

/// If `a` and `b` "face" each other along exactly one axis — separated by a
/// gap on that axis, with their projections overlapping on the perpendicular
/// axis — returns `(gap, run_length)`: the separation on the facing axis and
/// the length of the overlapping projection on the perpendicular axis (the
/// "parallel run length").
///
/// Returns `None` if the shapes overlap, only touch at a corner, or are
/// separated diagonally (a gap on both axes) — none of these have a single
/// well-defined facing axis.
fn facing_overlap(a: &Rect, b: &Rect) -> Option<(i64, i64)> {
    let x_gap = if a.x_hi <= b.x_lo {
        b.x_lo - a.x_hi
    } else if b.x_hi <= a.x_lo {
        a.x_lo - b.x_hi
    } else {
        0
    };
    let y_gap = if a.y_hi <= b.y_lo {
        b.y_lo - a.y_hi
    } else if b.y_hi <= a.y_lo {
        a.y_lo - b.y_hi
    } else {
        0
    };

    let x_overlap = (a.x_hi.min(b.x_hi) - a.x_lo.max(b.x_lo)).max(0);
    let y_overlap = (a.y_hi.min(b.y_hi) - a.y_lo.max(b.y_lo)).max(0);

    match (x_gap > 0, y_gap > 0) {
        (true, false) if y_overlap > 0 => Some((x_gap, y_overlap)),
        (false, true) if x_overlap > 0 => Some((y_gap, x_overlap)),
        _ => None,
    }
}

/// The per-side enclosure of `inner` within `outer`: `(left, right, bottom,
/// top)`. Each value is how far `outer`'s edge extends beyond `inner`'s
/// corresponding edge; negative if `inner` extends past `outer` on that side.
fn enclosure_sides(outer: &Rect, inner: &Rect) -> (i64, i64, i64, i64) {
    let left = inner.x_lo - outer.x_lo;
    let right = outer.x_hi - inner.x_hi;
    let bottom = inner.y_lo - outer.y_lo;
    let top = outer.y_hi - inner.y_hi;
    (left, right, bottom, top)
}

/// Basic spacing check between two rectangles on the same layer.
///
/// `Some(DrcViolation)` if the edge-to-edge distance between `a` and `b` is
/// less than `min_spacing` and they do not overlap. `None` if the spacing is
/// satisfied or the shapes overlap (overlap is [`check_overlap`]'s job).
///
/// The edge-to-edge distance is [`Rect::manhattan_distance`]: zero when the
/// rectangles' projections overlap on both axes (i.e. they touch or overlap),
/// and the sum of the per-axis gaps otherwise.
///
/// Symmetric: `check_spacing(a, b, ...) == check_spacing(b, a, ...)`.
#[allow(clippy::too_many_arguments)]
pub fn check_spacing(
    a: &Rect,
    b: &Rect,
    a_ref: ShapeRef,
    b_ref: ShapeRef,
    layer: LayerId,
    rule: RuleId,
    min_spacing: i64,
    same_net: bool,
) -> Option<DrcViolation> {
    check_spacing_like(
        a,
        b,
        a_ref,
        b_ref,
        layer,
        rule,
        min_spacing,
        same_net,
        ViolationKind::Spacing,
    )
}

/// Minimum width check for a single rectangle.
///
/// `Some(DrcViolation)` if `min(shape.width(), shape.height()) < min_width`.
/// Width is always the smaller dimension (orientation-independent); a square
/// shape with side `== min_width` passes.
pub fn check_width(
    shape: &Rect,
    shape_ref: ShapeRef,
    layer: LayerId,
    rule: RuleId,
    min_width: i64,
) -> Option<DrcViolation> {
    let actual = shape.width().min(shape.height());
    if actual < min_width {
        Some(DrcViolation {
            kind: ViolationKind::Width,
            rule,
            layer,
            shapes: (shape_ref, None),
            region: *shape,
            required: min_width,
            actual,
            same_net: false,
        })
    } else {
        None
    }
}

/// Minimum area check for a single rectangle.
///
/// `Some(DrcViolation)` if `shape.area() < min_area`. A shape with
/// `area == min_area` passes.
pub fn check_area(
    shape: &Rect,
    shape_ref: ShapeRef,
    layer: LayerId,
    rule: RuleId,
    min_area: i64,
) -> Option<DrcViolation> {
    let actual = shape.area();
    if actual < min_area {
        Some(DrcViolation {
            kind: ViolationKind::Area,
            rule,
            layer,
            shapes: (shape_ref, None),
            region: *shape,
            required: min_area,
            actual,
            same_net: false,
        })
    } else {
        None
    }
}

/// Overlap check: two shapes on the same layer must not overlap unless they
/// belong to the same net (merge semantics).
///
/// `Some(DrcViolation)` if `a` and `b` overlap with positive area and
/// `same_net` is false. `None` if they do not overlap, only touch, or
/// `same_net` is true (same-net shapes are merged, not flagged).
///
/// `required` is `0` (no overlap allowed); `actual` is the minimum extent of
/// the overlap region (how deep the overlap is), which is `> 0` whenever this
/// fires.
///
/// Symmetric: `check_overlap(a, b, ...) == check_overlap(b, a, ...)`.
pub fn check_overlap(
    a: &Rect,
    b: &Rect,
    a_ref: ShapeRef,
    b_ref: ShapeRef,
    layer: LayerId,
    rule: RuleId,
    same_net: bool,
) -> Option<DrcViolation> {
    if same_net {
        return None;
    }
    let overlap = a.intersection(b)?;
    Some(DrcViolation {
        kind: ViolationKind::Overlap,
        rule,
        layer,
        shapes: (a_ref, Some(b_ref)),
        region: overlap,
        required: 0,
        actual: overlap.width().min(overlap.height()),
        same_net,
    })
}

/// Grid snap check. All shape edges must land on the manufacturing grid.
///
/// `Some(DrcViolation)` if any edge coordinate of `shape` is not a multiple
/// of `grid_pitch`. `actual` is the worst-case offset from the nearest grid
/// line across all four edges; `required` is `0`.
///
/// `grid_pitch` must be `> 0` (panics in debug builds otherwise).
pub fn check_grid_snap(
    shape: &Rect,
    shape_ref: ShapeRef,
    layer: LayerId,
    rule: RuleId,
    grid_pitch: i64,
) -> Option<DrcViolation> {
    debug_assert!(grid_pitch > 0, "grid_pitch must be positive");

    let offset = |coord: i64| -> i64 {
        let rem = coord.rem_euclid(grid_pitch);
        rem.min(grid_pitch - rem)
    };

    let actual = offset(shape.x_lo)
        .max(offset(shape.y_lo))
        .max(offset(shape.x_hi))
        .max(offset(shape.y_hi));

    if actual != 0 {
        Some(DrcViolation {
            kind: ViolationKind::GridSnap,
            rule,
            layer,
            shapes: (shape_ref, None),
            region: *shape,
            required: 0,
            actual,
            same_net: false,
        })
    } else {
        None
    }
}

/// Outline exceedance check. A shape must lie entirely within the cell boundary.
///
/// `Some(DrcViolation)` if any part of `shape` extends beyond `boundary`.
/// `actual` is the maximum exceedance distance on any side; `required` is
/// `0`. A shape touching the boundary edge (but not exceeding it) passes.
pub fn check_outline_exceed(
    shape: &Rect,
    shape_ref: ShapeRef,
    boundary: &Rect,
    layer: LayerId,
    rule: RuleId,
) -> Option<DrcViolation> {
    let actual = (boundary.x_lo - shape.x_lo)
        .max(shape.x_hi - boundary.x_hi)
        .max(boundary.y_lo - shape.y_lo)
        .max(shape.y_hi - boundary.y_hi)
        .max(0);

    if actual > 0 {
        Some(DrcViolation {
            kind: ViolationKind::OutlineExceed,
            rule,
            layer,
            shapes: (shape_ref, None),
            region: bbox(shape, boundary),
            required: 0,
            actual,
            same_net: false,
        })
    } else {
        None
    }
}

/// Enclosure check: `outer` must enclose `inner` by at least `min_enclosure`
/// on all four sides.
///
/// `Some(DrcViolation)` if any edge of `inner` is closer than
/// `min_enclosure` to the corresponding edge of `outer`. If `inner` is
/// entirely outside `outer`, this still fires (enclosure is negative).
/// `actual` is the minimum per-side enclosure (left, right, bottom, top).
pub fn check_enclosure(
    outer: &Rect,
    inner: &Rect,
    outer_ref: ShapeRef,
    inner_ref: ShapeRef,
    layer: LayerId,
    rule: RuleId,
    min_enclosure: i64,
) -> Option<DrcViolation> {
    let (left, right, bottom, top) = enclosure_sides(outer, inner);
    let actual = left.min(right).min(bottom).min(top);
    if actual < min_enclosure {
        Some(DrcViolation {
            kind: ViolationKind::Enclosure,
            rule,
            layer,
            shapes: (outer_ref, Some(inner_ref)),
            region: bbox(outer, inner),
            required: min_enclosure,
            actual,
            same_net: false,
        })
    } else {
        None
    }
}

/// End-of-line (EOL) spacing check. When a metal edge terminates (its run
/// length is short), the required spacing to adjacent shapes increases.
///
/// A shape has an EOL edge if its narrower dimension is `<= eol_width`. If
/// neither `a` nor `b` has an EOL edge, returns `None` ([`check_spacing`]
/// applies instead). Otherwise, if `a` and `b` face each other (see
/// [`facing_overlap`]) with a gap `<= eol_within`, fires if that gap is less
/// than `eol_space`.
///
/// Symmetric in `a`, `b`.
#[allow(clippy::too_many_arguments)]
pub fn check_eol_spacing(
    a: &Rect,
    b: &Rect,
    a_ref: ShapeRef,
    b_ref: ShapeRef,
    layer: LayerId,
    rule: RuleId,
    eol_width: i64,
    eol_space: i64,
    eol_within: i64,
    same_net: bool,
) -> Option<DrcViolation> {
    let has_eol = |s: &Rect| s.width().min(s.height()) <= eol_width;
    if !has_eol(a) && !has_eol(b) {
        return None;
    }
    let (gap, _run_length) = facing_overlap(a, b)?;
    if gap <= eol_within && gap < eol_space {
        Some(DrcViolation {
            kind: ViolationKind::EOL,
            rule,
            layer,
            shapes: (a_ref, Some(b_ref)),
            region: bbox(a, b),
            required: eol_space,
            actual: gap,
            same_net,
        })
    } else {
        None
    }
}

/// Parallel run length (PRL) dependent spacing check. When two shapes run
/// parallel for a long enough distance, the required spacing increases.
///
/// `a` and `b` must face each other along exactly one axis (see
/// [`facing_overlap`]); if they don't (diagonal separation or overlap),
/// returns `None`. If the parallel run length is `< prl_threshold`, returns
/// `None` ([`check_spacing`] applies instead). Otherwise fires if the gap is
/// less than `prl_spacing`.
///
/// Symmetric in `a`, `b`.
#[allow(clippy::too_many_arguments)]
pub fn check_prl_spacing(
    a: &Rect,
    b: &Rect,
    a_ref: ShapeRef,
    b_ref: ShapeRef,
    layer: LayerId,
    rule: RuleId,
    prl_threshold: i64,
    prl_spacing: i64,
    same_net: bool,
) -> Option<DrcViolation> {
    let (gap, run_length) = facing_overlap(a, b)?;
    if run_length < prl_threshold {
        return None;
    }
    if gap < prl_spacing {
        Some(DrcViolation {
            kind: ViolationKind::PRL,
            rule,
            layer,
            shapes: (a_ref, Some(b_ref)),
            region: bbox(a, b),
            required: prl_spacing,
            actual: gap,
            same_net,
        })
    } else {
        None
    }
}

/// Via cut spacing check. Minimum spacing between two via cut shapes.
///
/// Same semantics as [`check_spacing`], but reported as
/// [`ViolationKind::CutSpacing`].
#[allow(clippy::too_many_arguments)]
pub fn check_cut_spacing(
    a: &Rect,
    b: &Rect,
    a_ref: ShapeRef,
    b_ref: ShapeRef,
    layer: LayerId,
    rule: RuleId,
    min_cut_spacing: i64,
    same_net: bool,
) -> Option<DrcViolation> {
    check_spacing_like(
        a,
        b,
        a_ref,
        b_ref,
        layer,
        rule,
        min_cut_spacing,
        same_net,
        ViolationKind::CutSpacing,
    )
}

/// Same-net notch check. A narrow channel (notch) between two same-net
/// shapes can cause manufacturing issues even though overlap is allowed.
///
/// `a` and `b` must face each other along exactly one axis (see
/// [`facing_overlap`]); if they don't (diagonal separation or overlap),
/// returns `None`. Fires if the gap (notch width) is less than
/// `min_notch_width` and the parallel run length (notch length) is less than
/// `max_notch_length`.
///
/// Only meaningful for same-net shapes (the caller must verify this); the
/// reported violation's `same_net` is always `true`.
///
/// Symmetric in `a`, `b`.
#[allow(clippy::too_many_arguments)]
pub fn check_same_net_notch(
    a: &Rect,
    b: &Rect,
    a_ref: ShapeRef,
    b_ref: ShapeRef,
    layer: LayerId,
    rule: RuleId,
    min_notch_width: i64,
    max_notch_length: i64,
) -> Option<DrcViolation> {
    let (gap, run_length) = facing_overlap(a, b)?;
    if gap < min_notch_width && run_length < max_notch_length {
        Some(DrcViolation {
            kind: ViolationKind::SameNetNotch,
            rule,
            layer,
            shapes: (a_ref, Some(b_ref)),
            region: bbox(a, b),
            required: min_notch_width,
            actual: gap,
            same_net: true,
        })
    } else {
        None
    }
}

/// Well-to-well spacing check. Minimum spacing between two well shapes.
///
/// Same geometry as [`check_spacing`], reported as
/// [`ViolationKind::WellSpacing`]. Does not apply to overlapping wells (those
/// are merged); `same_net` is always `false`.
pub fn check_well_spacing(
    a: &Rect,
    b: &Rect,
    a_ref: ShapeRef,
    b_ref: ShapeRef,
    layer: LayerId,
    rule: RuleId,
    min_well_spacing: i64,
) -> Option<DrcViolation> {
    check_spacing_like(
        a,
        b,
        a_ref,
        b_ref,
        layer,
        rule,
        min_well_spacing,
        false,
        ViolationKind::WellSpacing,
    )
}

/// Well enclosure of diffusion. `well` must extend beyond `diffusion` by at
/// least `min_well_enc` on all sides.
///
/// Delegates to the same enclosure math as [`check_enclosure`], reported as
/// [`ViolationKind::WellEnclosure`].
pub fn check_well_enclosure(
    well: &Rect,
    diffusion: &Rect,
    well_ref: ShapeRef,
    diff_ref: ShapeRef,
    layer: LayerId,
    rule: RuleId,
    min_well_enc: i64,
) -> Option<DrcViolation> {
    check_enclosure(
        well,
        diffusion,
        well_ref,
        diff_ref,
        layer,
        rule,
        min_well_enc,
    )
    .map(|mut v| {
        v.kind = ViolationKind::WellEnclosure;
        v
    })
}

/// Implant-to-implant spacing check. Minimum spacing between two implant
/// regions (e.g., NPlus to PPlus).
///
/// Same geometry as [`check_spacing`], reported as
/// [`ViolationKind::ImplantSpacing`]. `same_net` is always `false`.
pub fn check_implant_spacing(
    a: &Rect,
    b: &Rect,
    a_ref: ShapeRef,
    b_ref: ShapeRef,
    layer: LayerId,
    rule: RuleId,
    min_implant_spacing: i64,
) -> Option<DrcViolation> {
    check_spacing_like(
        a,
        b,
        a_ref,
        b_ref,
        layer,
        rule,
        min_implant_spacing,
        false,
        ViolationKind::ImplantSpacing,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x_lo: i64, y_lo: i64, x_hi: i64, y_hi: i64) -> Rect {
        Rect::new(x_lo, y_lo, x_hi, y_hi).unwrap()
    }

    fn rule(name: &str) -> RuleId {
        RuleId(name.to_string())
    }

    const LAYER: LayerId = LayerId(1);
    const A: ShapeRef = ShapeRef(0);
    const B: ShapeRef = ShapeRef(1);

    // --- check_spacing -----------------------------------------------------

    #[test]
    fn spacing_passes_at_exactly_min_spacing() {
        let a = rect(0, 0, 10, 10);
        let b = rect(15, 0, 25, 10);
        assert_eq!(
            check_spacing(&a, &b, A, B, LAYER, rule("S.1"), 5, false),
            None
        );
    }

    #[test]
    fn spacing_fails_when_too_close() {
        let a = rect(0, 0, 10, 10);
        let b = rect(13, 0, 23, 10);
        let v = check_spacing(&a, &b, A, B, LAYER, rule("S.1"), 5, false).unwrap();
        assert_eq!(v.kind, ViolationKind::Spacing);
        assert_eq!(v.required, 5);
        assert_eq!(v.actual, 3);
        assert!(!v.same_net);
    }

    #[test]
    fn spacing_symmetric() {
        let a = rect(0, 0, 10, 10);
        let b = rect(13, 0, 23, 10);
        let fwd = check_spacing(&a, &b, A, B, LAYER, rule("S.1"), 5, false);
        let rev = check_spacing(&b, &a, B, A, LAYER, rule("S.1"), 5, false);
        assert_eq!(fwd.map(|v| v.actual), rev.map(|v| v.actual));
    }

    #[test]
    fn spacing_does_not_fire_on_overlap() {
        let a = rect(0, 0, 10, 10);
        let b = rect(5, 5, 15, 15);
        assert_eq!(
            check_spacing(&a, &b, A, B, LAYER, rule("S.1"), 100, false),
            None
        );
    }

    // --- check_width --------------------------------------------------------

    #[test]
    fn width_passes_at_exactly_min_width() {
        let shape = rect(0, 0, 5, 5);
        assert_eq!(check_width(&shape, A, LAYER, rule("W.1"), 5), None);
    }

    #[test]
    fn width_fails_when_too_narrow() {
        let shape = rect(0, 0, 10, 3);
        let v = check_width(&shape, A, LAYER, rule("W.1"), 5).unwrap();
        assert_eq!(v.kind, ViolationKind::Width);
        assert_eq!(v.required, 5);
        assert_eq!(v.actual, 3);
        assert_eq!(v.shapes, (A, None));
    }

    #[test]
    fn width_is_orientation_independent() {
        let tall = rect(0, 0, 3, 10);
        let wide = rect(0, 0, 10, 3);
        assert_eq!(
            check_width(&tall, A, LAYER, rule("W.1"), 5).unwrap().actual,
            check_width(&wide, A, LAYER, rule("W.1"), 5).unwrap().actual,
        );
    }

    // --- check_area ----------------------------------------------------------

    #[test]
    fn area_passes_at_exactly_min_area() {
        let shape = rect(0, 0, 4, 4);
        assert_eq!(check_area(&shape, A, LAYER, rule("A.1"), 16), None);
    }

    #[test]
    fn area_fails_when_too_small() {
        let shape = rect(0, 0, 3, 3);
        let v = check_area(&shape, A, LAYER, rule("A.1"), 16).unwrap();
        assert_eq!(v.kind, ViolationKind::Area);
        assert_eq!(v.required, 16);
        assert_eq!(v.actual, 9);
    }

    // --- check_overlap ---------------------------------------------------------

    #[test]
    fn overlap_fires_on_positive_area_overlap() {
        let a = rect(0, 0, 10, 10);
        let b = rect(5, 5, 15, 15);
        let v = check_overlap(&a, &b, A, B, LAYER, rule("O.1"), false).unwrap();
        assert_eq!(v.kind, ViolationKind::Overlap);
        assert_eq!(v.required, 0);
        assert_eq!(v.actual, 5);
    }

    #[test]
    fn overlap_does_not_fire_when_touching() {
        let a = rect(0, 0, 10, 10);
        let b = rect(10, 0, 20, 10);
        assert_eq!(check_overlap(&a, &b, A, B, LAYER, rule("O.1"), false), None);
    }

    #[test]
    fn overlap_does_not_fire_for_same_net() {
        let a = rect(0, 0, 10, 10);
        let b = rect(5, 5, 15, 15);
        assert_eq!(check_overlap(&a, &b, A, B, LAYER, rule("O.1"), true), None);
    }

    #[test]
    fn overlap_symmetric() {
        let a = rect(0, 0, 10, 10);
        let b = rect(5, 5, 15, 15);
        let fwd = check_overlap(&a, &b, A, B, LAYER, rule("O.1"), false).unwrap();
        let rev = check_overlap(&b, &a, B, A, LAYER, rule("O.1"), false).unwrap();
        assert_eq!(fwd.region, rev.region);
        assert_eq!(fwd.actual, rev.actual);
    }

    // --- check_grid_snap -----------------------------------------------------

    #[test]
    fn grid_snap_passes_on_grid() {
        let shape = rect(0, 10, 50, 60);
        assert_eq!(check_grid_snap(&shape, A, LAYER, rule("G.1"), 5), None);
    }

    #[test]
    fn grid_snap_fails_off_grid() {
        let shape = rect(0, 0, 11, 10);
        let v = check_grid_snap(&shape, A, LAYER, rule("G.1"), 5).unwrap();
        assert_eq!(v.kind, ViolationKind::GridSnap);
        assert_eq!(v.required, 0);
        assert_eq!(v.actual, 1);
    }

    #[test]
    fn grid_snap_handles_negative_coordinates() {
        let shape = rect(-10, -5, 0, 5);
        assert_eq!(check_grid_snap(&shape, A, LAYER, rule("G.1"), 5), None);

        let off = rect(-11, -5, 0, 5);
        let v = check_grid_snap(&off, A, LAYER, rule("G.1"), 5).unwrap();
        assert_eq!(v.actual, 1);
    }

    // --- check_outline_exceed --------------------------------------------------

    #[test]
    fn outline_exceed_passes_inside_boundary() {
        let boundary = rect(0, 0, 100, 100);
        let shape = rect(0, 0, 100, 100);
        assert_eq!(
            check_outline_exceed(&shape, A, &boundary, LAYER, rule("B.1")),
            None
        );
    }

    #[test]
    fn outline_exceed_fires_when_outside() {
        let boundary = rect(0, 0, 100, 100);
        let shape = rect(90, 90, 110, 95);
        let v = check_outline_exceed(&shape, A, &boundary, LAYER, rule("B.1")).unwrap();
        assert_eq!(v.kind, ViolationKind::OutlineExceed);
        assert_eq!(v.required, 0);
        assert_eq!(v.actual, 10);
        assert_eq!(v.shapes, (A, None));
    }

    // --- check_enclosure ------------------------------------------------------

    #[test]
    fn enclosure_passes_at_exactly_min_enclosure() {
        let outer = rect(0, 0, 20, 20);
        let inner = rect(2, 2, 18, 18);
        assert_eq!(
            check_enclosure(&outer, &inner, A, B, LAYER, rule("E.1"), 2),
            None
        );
    }

    #[test]
    fn enclosure_fails_when_too_close_to_edge() {
        let outer = rect(0, 0, 20, 20);
        let inner = rect(1, 2, 18, 18);
        let v = check_enclosure(&outer, &inner, A, B, LAYER, rule("E.1"), 2).unwrap();
        assert_eq!(v.kind, ViolationKind::Enclosure);
        assert_eq!(v.required, 2);
        assert_eq!(v.actual, 1);
        assert_eq!(v.shapes, (A, Some(B)));
    }

    #[test]
    fn enclosure_fires_negative_when_inner_outside_outer() {
        let outer = rect(0, 0, 10, 10);
        let inner = rect(8, 8, 20, 20);
        let v = check_enclosure(&outer, &inner, A, B, LAYER, rule("E.1"), 2).unwrap();
        assert_eq!(v.kind, ViolationKind::Enclosure);
        assert!(v.actual < 0);
    }

    // --- facing_overlap / check_prl_spacing -------------------------------------

    #[test]
    fn prl_does_not_fire_below_threshold() {
        let a = rect(0, 0, 10, 5);
        let b = rect(13, 0, 23, 5);
        // run length (y overlap) is 5, threshold is 10 -> below threshold
        assert_eq!(
            check_prl_spacing(&a, &b, A, B, LAYER, rule("P.1"), 10, 5, false),
            None
        );
    }

    #[test]
    fn prl_fires_when_run_length_meets_threshold_and_spacing_too_small() {
        let a = rect(0, 0, 10, 20);
        let b = rect(13, 0, 23, 20);
        let v = check_prl_spacing(&a, &b, A, B, LAYER, rule("P.1"), 10, 5, false).unwrap();
        assert_eq!(v.kind, ViolationKind::PRL);
        assert_eq!(v.required, 5);
        assert_eq!(v.actual, 3);
    }

    #[test]
    fn prl_symmetric() {
        let a = rect(0, 0, 10, 20);
        let b = rect(13, 0, 23, 20);
        let fwd = check_prl_spacing(&a, &b, A, B, LAYER, rule("P.1"), 10, 5, false);
        let rev = check_prl_spacing(&b, &a, B, A, LAYER, rule("P.1"), 10, 5, false);
        assert_eq!(fwd.map(|v| v.actual), rev.map(|v| v.actual));
    }

    #[test]
    fn prl_returns_none_for_diagonal_separation() {
        let a = rect(0, 0, 10, 10);
        let b = rect(15, 15, 25, 25);
        assert_eq!(
            check_prl_spacing(&a, &b, A, B, LAYER, rule("P.1"), 0, 100, false),
            None
        );
    }

    // --- check_cut_spacing -----------------------------------------------------

    #[test]
    fn cut_spacing_same_semantics_as_spacing() {
        let a = rect(0, 0, 10, 10);
        let b = rect(13, 0, 23, 10);
        let v = check_cut_spacing(&a, &b, A, B, LAYER, rule("C.1"), 5, false).unwrap();
        assert_eq!(v.kind, ViolationKind::CutSpacing);
        assert_eq!(v.required, 5);
        assert_eq!(v.actual, 3);
    }

    // --- check_same_net_notch ----------------------------------------------------

    #[test]
    fn same_net_notch_fires_for_narrow_short_channel() {
        let a = rect(0, 0, 10, 20);
        let b = rect(11, 0, 21, 20);
        let v = check_same_net_notch(&a, &b, A, B, LAYER, rule("N.1"), 2, 30).unwrap();
        assert_eq!(v.kind, ViolationKind::SameNetNotch);
        assert_eq!(v.required, 2);
        assert_eq!(v.actual, 1);
        assert!(v.same_net);
    }

    #[test]
    fn same_net_notch_does_not_fire_when_notch_too_long() {
        let a = rect(0, 0, 10, 50);
        let b = rect(11, 0, 21, 50);
        assert_eq!(
            check_same_net_notch(&a, &b, A, B, LAYER, rule("N.1"), 2, 30),
            None
        );
    }

    #[test]
    fn same_net_notch_does_not_fire_when_wide_enough() {
        let a = rect(0, 0, 10, 20);
        let b = rect(15, 0, 25, 20);
        assert_eq!(
            check_same_net_notch(&a, &b, A, B, LAYER, rule("N.1"), 2, 30),
            None
        );
    }

    // --- check_well_spacing / check_implant_spacing -------------------------------

    #[test]
    fn well_spacing_same_semantics_as_spacing() {
        let a = rect(0, 0, 10, 10);
        let b = rect(13, 0, 23, 10);
        let v = check_well_spacing(&a, &b, A, B, LAYER, rule("WS.1"), 5).unwrap();
        assert_eq!(v.kind, ViolationKind::WellSpacing);
        assert_eq!(v.actual, 3);
        assert!(!v.same_net);
    }

    #[test]
    fn implant_spacing_same_semantics_as_spacing() {
        let a = rect(0, 0, 10, 10);
        let b = rect(13, 0, 23, 10);
        let v = check_implant_spacing(&a, &b, A, B, LAYER, rule("IS.1"), 5).unwrap();
        assert_eq!(v.kind, ViolationKind::ImplantSpacing);
        assert_eq!(v.actual, 3);
        assert!(!v.same_net);
    }

    // --- check_well_enclosure ----------------------------------------------------

    #[test]
    fn well_enclosure_delegates_to_enclosure_math() {
        let well = rect(0, 0, 20, 20);
        let diffusion = rect(1, 2, 18, 18);
        let v = check_well_enclosure(&well, &diffusion, A, B, LAYER, rule("WE.1"), 2).unwrap();
        assert_eq!(v.kind, ViolationKind::WellEnclosure);
        assert_eq!(v.required, 2);
        assert_eq!(v.actual, 1);
    }

    // --- check_eol_spacing -------------------------------------------------------

    #[test]
    fn eol_spacing_does_not_fire_without_eol_edge() {
        // Neither shape is narrow enough to have an EOL edge.
        let a = rect(0, 0, 10, 10);
        let b = rect(11, 0, 21, 10);
        assert_eq!(
            check_eol_spacing(&a, &b, A, B, LAYER, rule("EOL.1"), 2, 5, 10, false),
            None
        );
    }

    #[test]
    fn eol_spacing_fires_for_narrow_shape_within_range() {
        // `a` is narrow (height 2 <= eol_width 2), so it has an EOL edge.
        let a = rect(0, 0, 10, 2);
        let b = rect(11, 0, 21, 2);
        let v = check_eol_spacing(&a, &b, A, B, LAYER, rule("EOL.1"), 2, 5, 10, false).unwrap();
        assert_eq!(v.kind, ViolationKind::EOL);
        assert_eq!(v.required, 5);
        assert_eq!(v.actual, 1);
    }

    #[test]
    fn eol_spacing_does_not_fire_beyond_within_distance() {
        let a = rect(0, 0, 10, 2);
        let b = rect(20, 0, 30, 2);
        assert_eq!(
            check_eol_spacing(&a, &b, A, B, LAYER, rule("EOL.1"), 2, 50, 5, false),
            None
        );
    }
}
