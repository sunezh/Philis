//! Static IR drop analysis over a resistive power/ground mesh.
//!
//! See `crates/verify/PLAN.md` §6.2 for the full design.

use std::collections::{HashMap, HashSet};

/// Maximum number of Gauss-Seidel sweeps before giving up.
const MAX_ITERATIONS: usize = 10_000;

/// Convergence tolerance on the largest per-node voltage update (volts).
const TOLERANCE: f64 = 1e-12;

/// Compute static IR drop across a resistive network.
///
/// Model: the power/ground network is a resistive mesh. Each wire segment or
/// via is a resistor (from [`crate::wire_resistance`]/[`crate::via_resistance`]).
/// Current sources are placed at device terminals. `vdd_nodes` are held at
/// `vdd_voltage` (ideal voltage sources); every other node's voltage is solved
/// for via nodal analysis: for node `i` with neighbors `j` and conductances
/// `G_ij = 1/R_ij`,
///
/// ```text
/// V_i = (sum_j G_ij * V_j - I_i) / sum_j G_ij
/// ```
///
/// where `I_i` is the net current injected by current sources at node `i`
/// (positive = sink, i.e. current flowing out of the node into a device).
/// This is solved iteratively via Gauss-Seidel, which converges for the
/// diagonally-dominant conductance matrices that resistive meshes produce.
///
/// Inputs:
///   - `nodes`: list of node ids in the resistive network
///   - `resistors`: list of `(node_a, node_b, resistance_ohms)`
///   - `current_sources`: list of `(node, current_amps)` — positive = current sink
///   - `vdd_nodes`: nodes held at `vdd_voltage` (ideal voltage sources)
///   - `vdd_voltage`: the supply voltage
///
/// Nodes with no resistors attached (isolated from the mesh) are left at
/// `0.0` volts, since their voltage is undetermined by the network.
///
/// # Panics
///
/// In debug builds, panics if any resistance in `resistors` is `<= 0.0`.
pub fn compute_ir_drop(
    nodes: &[u32],
    resistors: &[(u32, u32, f64)],
    current_sources: &[(u32, f64)],
    vdd_nodes: &[u32],
    vdd_voltage: f64,
) -> IrDropResult {
    let vdd_set: HashSet<u32> = vdd_nodes.iter().copied().collect();

    let mut diag_conductance: HashMap<u32, f64> = nodes.iter().map(|&n| (n, 0.0)).collect();
    let mut adjacency: HashMap<u32, Vec<(u32, f64)>> =
        nodes.iter().map(|&n| (n, Vec::new())).collect();
    for &(a, b, r) in resistors {
        debug_assert!(r > 0.0, "compute_ir_drop: resistance must be positive");
        let g = 1.0 / r;
        *diag_conductance.entry(a).or_insert(0.0) += g;
        *diag_conductance.entry(b).or_insert(0.0) += g;
        adjacency.entry(a).or_default().push((b, g));
        adjacency.entry(b).or_default().push((a, g));
    }

    let mut injected: HashMap<u32, f64> = HashMap::new();
    for &(n, i) in current_sources {
        *injected.entry(n).or_insert(0.0) += i;
    }

    let mut voltages: HashMap<u32, f64> = nodes
        .iter()
        .map(|&n| {
            (
                n,
                if vdd_set.contains(&n) {
                    vdd_voltage
                } else {
                    0.0
                },
            )
        })
        .collect();

    let mut converged = false;
    for _ in 0..MAX_ITERATIONS {
        let mut max_delta = 0.0_f64;
        for &n in nodes {
            if vdd_set.contains(&n) {
                continue;
            }
            let g_ii = diag_conductance[&n];
            if g_ii == 0.0 {
                continue;
            }
            let neighbor_sum: f64 = adjacency[&n].iter().map(|&(m, g)| g * voltages[&m]).sum();
            let i = injected.get(&n).copied().unwrap_or(0.0);
            let v_new = (neighbor_sum - i) / g_ii;
            let delta = (v_new - voltages[&n]).abs();
            if delta > max_delta {
                max_delta = delta;
            }
            voltages.insert(n, v_new);
        }
        if max_delta < TOLERANCE {
            converged = true;
            break;
        }
    }

    let node_voltages: Vec<(u32, f64)> = nodes.iter().map(|&n| (n, voltages[&n])).collect();

    let mut worst_drop = 0.0;
    let mut worst_node = nodes.first().copied().unwrap_or(0);
    for &(n, v) in &node_voltages {
        let drop = vdd_voltage - v;
        if drop > worst_drop {
            worst_drop = drop;
            worst_node = n;
        }
    }

    IrDropResult {
        node_voltages,
        worst_drop,
        worst_node,
        converged,
    }
}

/// Result of [`compute_ir_drop`].
#[derive(Debug, Clone)]
pub struct IrDropResult {
    /// Voltage at each node, in the same order as the input `nodes` slice.
    pub node_voltages: Vec<(u32, f64)>,
    /// Worst-case voltage drop (`vdd_voltage - V_node`, maximized over all
    /// nodes; `0.0` if `nodes` is empty).
    pub worst_drop: f64,
    /// Node with the worst drop (the first node, or `0`, if `nodes` is empty).
    pub worst_node: u32,
    /// Whether the Gauss-Seidel iteration converged within
    /// [`MAX_ITERATIONS`].
    pub converged: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} != {b}");
    }

    #[test]
    fn single_resistor_voltage_divider() {
        // VDD(0) --10ohm-- node 1, with a 0.01A sink at node 1.
        let result = compute_ir_drop(&[0, 1], &[(0, 1, 10.0)], &[(1, 0.01)], &[0], 1.0);

        assert!(result.converged);
        let v1 = result
            .node_voltages
            .iter()
            .find(|&&(n, _)| n == 1)
            .unwrap()
            .1;
        assert_close(v1, 1.0 - 0.01 * 10.0);
        assert_close(result.worst_drop, 0.1);
        assert_eq!(result.worst_node, 1);
    }

    #[test]
    fn chain_network_accumulates_drop() {
        // VDD(0) --10ohm-- node 1 --10ohm-- node 2, with a 0.01A sink at node 2.
        let result = compute_ir_drop(
            &[0, 1, 2],
            &[(0, 1, 10.0), (1, 2, 10.0)],
            &[(2, 0.01)],
            &[0],
            1.0,
        );

        assert!(result.converged);
        let v1 = result
            .node_voltages
            .iter()
            .find(|&&(n, _)| n == 1)
            .unwrap()
            .1;
        let v2 = result
            .node_voltages
            .iter()
            .find(|&&(n, _)| n == 2)
            .unwrap()
            .1;
        assert_close(v1, 0.9);
        assert_close(v2, 0.8);
        assert_close(result.worst_drop, 0.2);
        assert_eq!(result.worst_node, 2);
    }

    #[test]
    fn symmetric_dual_supply_splits_current() {
        // VDD(0) --10ohm-- node 1 --10ohm-- VDD(2), with a 0.02A sink at node 1.
        let result = compute_ir_drop(
            &[0, 1, 2],
            &[(0, 1, 10.0), (2, 1, 10.0)],
            &[(1, 0.02)],
            &[0, 2],
            1.0,
        );

        assert!(result.converged);
        let v1 = result
            .node_voltages
            .iter()
            .find(|&&(n, _)| n == 1)
            .unwrap()
            .1;
        // Current splits evenly between the two supplies: 0.01A through each.
        assert_close(v1, 1.0 - 0.01 * 10.0);
        assert_close(result.worst_drop, 0.1);
        assert_eq!(result.worst_node, 1);
    }

    #[test]
    fn all_vdd_nodes_with_no_current_is_zero_drop() {
        let result = compute_ir_drop(&[0, 1], &[(0, 1, 5.0)], &[], &[0, 1], 1.0);

        assert!(result.converged);
        assert_close(result.worst_drop, 0.0);
    }

    #[test]
    fn floating_node_defaults_to_zero_volts() {
        // Node 2 has no resistors at all.
        let result = compute_ir_drop(&[0, 1, 2], &[(0, 1, 5.0)], &[], &[0], 1.0);

        assert!(result.converged);
        let v2 = result
            .node_voltages
            .iter()
            .find(|&&(n, _)| n == 2)
            .unwrap()
            .1;
        assert_close(v2, 0.0);
        // The floating node has the largest drop from VDD.
        assert_eq!(result.worst_node, 2);
        assert_close(result.worst_drop, 1.0);
    }

    #[test]
    #[should_panic]
    fn panics_on_non_positive_resistance_in_debug() {
        compute_ir_drop(&[0, 1], &[(0, 1, 0.0)], &[], &[0], 1.0);
    }
}
