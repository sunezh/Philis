//! Union-find with net labels for LVS connectivity extraction.
//!
//! See `crates/verify/PLAN.md` §4.1-4.2 for the full design.

use std::cmp::Ordering;
use std::collections::HashMap;

use super::{LvsError, LvsOpen, LvsShort};

/// Opaque net identifier from the schematic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetId(pub u32);

/// Result of a union operation.
#[derive(Debug, Clone, PartialEq)]
pub enum UnionResult {
    /// Both elements were already in the same component.
    AlreadySame,
    /// Merged two components with compatible labels (or at least one unlabeled).
    Merged,
    /// Merged two components with conflicting labels — this is a short.
    Short { net_a: NetId, net_b: NetId },
}

/// Union-find with net labels for LVS connectivity extraction.
///
/// Elements are identified by `u32` indices. Each element may optionally
/// carry a [`NetId`] label from the schematic. The tracker supports union,
/// find, same-component queries, and invariant checking.
#[derive(Debug, Clone)]
pub struct ConnectivityTracker {
    parent: Vec<u32>,
    rank: Vec<u8>,
    labels: Vec<Option<NetId>>,
    count: u32,
}

impl ConnectivityTracker {
    /// Create a new tracker with `n` elements, each in its own component.
    pub fn new(n: u32) -> Self {
        ConnectivityTracker {
            parent: (0..n).collect(),
            rank: vec![0; n as usize],
            labels: vec![None; n as usize],
            count: n,
        }
    }

    /// Assign a net label to element `x`.
    ///
    /// # Panics
    ///
    /// Panics if `x` already has a different label (label conflicts are LVS
    /// errors, caught earlier).
    pub fn label(&mut self, x: u32, net: NetId) {
        match self.labels[x as usize] {
            Some(existing) if existing != net => {
                panic!("element {x} already labeled {existing:?}, cannot relabel to {net:?}");
            }
            _ => self.labels[x as usize] = Some(net),
        }
    }

    /// Find the representative of element `x` with path compression.
    pub fn find(&mut self, x: u32) -> u32 {
        if self.parent[x as usize] != x {
            let root = self.find(self.parent[x as usize]);
            self.parent[x as usize] = root;
        }
        self.parent[x as usize]
    }

    /// Check whether `x` and `y` are in the same component.
    pub fn same_component(&mut self, x: u32, y: u32) -> bool {
        self.find(x) == self.find(y)
    }

    /// Union elements `x` and `y` into the same component.
    /// Uses union-by-rank with path compression.
    pub fn union(&mut self, x: u32, y: u32) -> UnionResult {
        let net_x = self.component_net(x);
        let net_y = self.component_net(y);
        let rx = self.find(x);
        let ry = self.find(y);

        if rx == ry {
            return UnionResult::AlreadySame;
        }

        match self.rank[rx as usize].cmp(&self.rank[ry as usize]) {
            Ordering::Less => self.parent[rx as usize] = ry,
            Ordering::Greater => self.parent[ry as usize] = rx,
            Ordering::Equal => {
                self.parent[ry as usize] = rx;
                self.rank[rx as usize] += 1;
            }
        }
        self.count -= 1;

        match (net_x, net_y) {
            (Some(a), Some(b)) if a != b => UnionResult::Short { net_a: a, net_b: b },
            _ => UnionResult::Merged,
        }
    }

    /// Return all elements in the same component as `x`.
    pub fn component_members(&mut self, x: u32) -> Vec<u32> {
        let root = self.find(x);
        (0..self.parent.len() as u32)
            .filter(|&i| self.find(i) == root)
            .collect()
    }

    /// Return the net label of the component containing `x`, if any.
    pub fn component_net(&mut self, x: u32) -> Option<NetId> {
        let root = self.find(x);
        (0..self.parent.len() as u32).find_map(|i| {
            if self.find(i) == root {
                self.labels[i as usize]
            } else {
                None
            }
        })
    }

    /// Return all elements labeled with `net`.
    pub fn elements_with_net(&self, net: NetId) -> Vec<u32> {
        self.labels
            .iter()
            .enumerate()
            .filter_map(|(i, label)| (*label == Some(net)).then_some(i as u32))
            .collect()
    }

    /// Invariant check: for every net N, all elements labeled N must be in
    /// the same component, and no component may contain elements with
    /// different net labels. Returns a list of violations.
    pub fn invariant_check(&mut self) -> Vec<LvsError> {
        let n = self.parent.len() as u32;
        let mut by_root: HashMap<u32, Vec<u32>> = HashMap::new();
        for i in 0..n {
            let root = self.find(i);
            by_root.entry(root).or_default().push(i);
        }

        let mut errors = Vec::new();

        let mut nets: Vec<NetId> = self.labels.iter().filter_map(|l| *l).collect();
        nets.sort_by_key(|net| net.0);
        nets.dedup();

        for net in nets {
            let mut roots: Vec<u32> = self
                .elements_with_net(net)
                .into_iter()
                .map(|e| self.find(e))
                .collect();
            roots.sort_unstable();
            roots.dedup();
            if roots.len() > 1 {
                let components = roots.iter().map(|r| by_root[r].clone()).collect();
                errors.push(LvsError::Open(LvsOpen { net, components }));
            }
        }

        for (&root, members) in &by_root {
            let mut nets_in_component: Vec<NetId> = members
                .iter()
                .filter_map(|&e| self.labels[e as usize])
                .collect();
            nets_in_component.sort_by_key(|net| net.0);
            nets_in_component.dedup();
            if nets_in_component.len() > 1 {
                errors.push(LvsError::Short(LvsShort {
                    nets: nets_in_component,
                    component: root,
                }));
            }
        }

        errors
    }

    /// Number of distinct components.
    pub fn component_count(&self) -> u32 {
        self.count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tracker_has_n_singleton_components() {
        let tracker = ConnectivityTracker::new(5);
        assert_eq!(tracker.component_count(), 5);
    }

    #[test]
    fn union_merges_components_and_decreases_count() {
        let mut tracker = ConnectivityTracker::new(3);
        assert_eq!(tracker.union(0, 1), UnionResult::Merged);
        assert_eq!(tracker.component_count(), 2);
        assert!(tracker.same_component(0, 1));
        assert!(!tracker.same_component(0, 2));
    }

    #[test]
    fn union_of_already_same_component_is_noop() {
        let mut tracker = ConnectivityTracker::new(2);
        tracker.union(0, 1);
        assert_eq!(tracker.union(0, 1), UnionResult::AlreadySame);
        assert_eq!(tracker.component_count(), 1);
    }

    #[test]
    fn transitive_union_puts_all_elements_in_same_component() {
        let mut tracker = ConnectivityTracker::new(4);
        tracker.union(0, 1);
        tracker.union(1, 2);
        tracker.union(2, 3);

        assert!(tracker.same_component(0, 3));
        assert_eq!(tracker.component_members(0).len(), 4);
        assert_eq!(tracker.component_count(), 1);
    }

    #[test]
    fn union_with_compatible_labels_merges() {
        let mut tracker = ConnectivityTracker::new(2);
        tracker.label(0, NetId(1));
        assert_eq!(tracker.union(0, 1), UnionResult::Merged);
        assert_eq!(tracker.component_net(1), Some(NetId(1)));
    }

    #[test]
    fn union_with_conflicting_labels_reports_short() {
        let mut tracker = ConnectivityTracker::new(2);
        tracker.label(0, NetId(1));
        tracker.label(1, NetId(2));

        match tracker.union(0, 1) {
            UnionResult::Short { net_a, net_b } => {
                assert_eq!(net_a, NetId(1));
                assert_eq!(net_b, NetId(2));
            }
            other => panic!("expected Short, got {other:?}"),
        }
    }

    #[test]
    #[should_panic]
    fn relabeling_with_different_net_panics() {
        let mut tracker = ConnectivityTracker::new(1);
        tracker.label(0, NetId(1));
        tracker.label(0, NetId(2));
    }

    #[test]
    fn relabeling_with_same_net_is_allowed() {
        let mut tracker = ConnectivityTracker::new(1);
        tracker.label(0, NetId(1));
        tracker.label(0, NetId(1));
        assert_eq!(tracker.component_net(0), Some(NetId(1)));
    }

    #[test]
    fn elements_with_net_finds_all_labeled_elements() {
        let mut tracker = ConnectivityTracker::new(3);
        tracker.label(0, NetId(1));
        tracker.label(2, NetId(1));
        tracker.label(1, NetId(2));

        let mut elems = tracker.elements_with_net(NetId(1));
        elems.sort_unstable();
        assert_eq!(elems, vec![0, 2]);
    }

    #[test]
    fn invariant_check_detects_open() {
        let mut tracker = ConnectivityTracker::new(2);
        tracker.label(0, NetId(1));
        tracker.label(1, NetId(1));
        // Not unioned: an "open" since both are labeled NetId(1) but live in
        // different components.

        let errors = tracker.invariant_check();
        assert_eq!(errors.len(), 1);
        match &errors[0] {
            LvsError::Open(open) => {
                assert_eq!(open.net, NetId(1));
                assert_eq!(open.components.len(), 2);
            }
            other => panic!("expected Open, got {other:?}"),
        }
    }

    #[test]
    fn invariant_check_detects_short() {
        let mut tracker = ConnectivityTracker::new(2);
        tracker.label(0, NetId(1));
        tracker.label(1, NetId(2));
        tracker.union(0, 1);

        let errors = tracker.invariant_check();
        assert_eq!(errors.len(), 1);
        match &errors[0] {
            LvsError::Short(short) => {
                assert_eq!(short.nets, vec![NetId(1), NetId(2)]);
            }
            other => panic!("expected Short, got {other:?}"),
        }
    }

    #[test]
    fn invariant_check_is_empty_for_consistent_labeling() {
        let mut tracker = ConnectivityTracker::new(4);
        tracker.label(0, NetId(1));
        tracker.label(1, NetId(1));
        tracker.label(2, NetId(2));
        tracker.label(3, NetId(2));
        tracker.union(0, 1);
        tracker.union(2, 3);

        assert!(tracker.invariant_check().is_empty());
    }
}
