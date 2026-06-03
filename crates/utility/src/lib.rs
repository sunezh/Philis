//! Shared generics and macros for the Philis crates.
//!
//! Per the crate's charter this holds **only** generics and macros — no real
//! implementation. The other crates pull these in to express common, data-
//! oriented patterns once. See `docs/Developer/general/*`.

/// The inclusive `[min, max]` extent of a set of values along one axis.
///
/// Returns `None` for an empty iterator. Generic over any `Copy + PartialOrd`
/// element, so it works for the integer grid coordinates in `api` as well as
/// floats, indices, etc.
pub fn bounds<I, T>(iter: I) -> Option<(T, T)>
where
    I: IntoIterator<Item = T>,
    T: Copy + PartialOrd,
{
    let mut it = iter.into_iter();
    let first = it.next()?;
    let (mut lo, mut hi) = (first, first);
    for v in it {
        if v < lo {
            lo = v;
        }
        if v > hi {
            hi = v;
        }
    }
    Some((lo, hi))
}

/// Project an iterator of items onto one field/axis and return its `[min, max]`
/// extent. Pairs with [`bounds`] to keep call sites in `api` free of manual
/// min/max bookkeeping.
///
/// ```
/// let pts = [(0, 3), (10, 1), (4, 8)];
/// assert_eq!(utility::extent!(pts.iter(), |p| p.0), Some((0, 10)));
/// assert_eq!(utility::extent!(pts.iter(), |p| p.1), Some((1, 8)));
/// ```
#[macro_export]
macro_rules! extent {
    ($iter:expr, $proj:expr) => {
        $crate::bounds($iter.map($proj))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_of_empty_is_none() {
        assert_eq!(bounds(std::iter::empty::<i64>()), None);
    }

    #[test]
    fn bounds_of_values() {
        assert_eq!(bounds([3, -2, 7, 0]), Some((-2, 7)));
    }

    #[test]
    fn extent_projects_a_field() {
        let pts = [(0, 3), (10, 1), (4, 8)];
        assert_eq!(extent!(pts.iter(), |p| p.0), Some((0, 10)));
        assert_eq!(extent!(pts.iter(), |p| p.1), Some((1, 8)));
    }
}
