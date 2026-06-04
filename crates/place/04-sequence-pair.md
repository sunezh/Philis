# Solver — Sequence-Pair + Simulated Annealing

## Why sequence-pair

The sequence-pair is the best first encoding for analog placement because:

1. **Every sequence-pair is a legal non-overlapping packing.** No overlap
   penalty needed — non-overlap is structural, not a cost term.
2. **Packing is a simple longest-path computation.** O(n log n) to decode
   a sequence-pair into (x, y) coordinates.
3. **Symmetry constraints map naturally.** A symmetric pair (A, A') can be
   encoded as a mirror constraint on their positions in the two sequences.
4. **The search space is well-understood.** The neighborhood (swap, insert)
   has been studied extensively.

The alternative B*-tree has cheaper per-move cost but doesn't encode symmetry
natively — you need a symmetric-feasible B*-tree variant, which is more complex
to implement correctly.

## The sequence-pair representation

A sequence-pair for n devices is a pair of permutations (Gamma+, Gamma-) of
the device indices {0, 1, ..., n-1}.

The relative position of two devices A, B is determined by their order in the
two sequences:

| Gamma+ order | Gamma- order | Geometric relation |
|-------------|-------------|-------------------|
| A before B | A before B | A is LEFT of B |
| A before B | B before A | A is BELOW B |
| B before A | A before B | A is ABOVE B |
| B before A | B before A | A is RIGHT of B |

### Decoding to coordinates

Given (Gamma+, Gamma-) and device widths w[i] and heights h[i]:

1. Build the **horizontal constraint graph** G_h:
   For each pair (A, B) where A is left of B, add edge A -> B with weight w[A].

2. Build the **vertical constraint graph** G_v:
   For each pair (A, B) where A is below B, add edge A -> B with weight h[A].

3. **x[i]** = longest path from source to i in G_h.
4. **y[i]** = longest path from source to i in G_v.

The longest-path computation uses topological sort (the permutation order gives
a valid topological order), so it's O(n) per graph after an O(n log n) sort.

### Efficient implementation

Rather than building full constraint graphs, use the **O(n log n) LCS-based
decoding**:

1. Compute `match[i]` = position of device i in Gamma-.
2. Process Gamma+ left to right. For each device i at position p+:
   - Its `match[i]` gives position in Gamma-.
   - x[i] = max weighted prefix query on positions 0..match[i] in Gamma-,
     weighted by width. Use a segment tree or BIT.
   - Similarly for y[i] using the reverse condition.

This gives O(n log n) decoding, which is critical because the SA loop calls
it thousands of times.

## The SA loop

### State

| Field | Type | Description |
|-------|------|-------------|
| `gamma_plus` | `Vec<usize>` | First permutation |
| `gamma_minus` | `Vec<usize>` | Second permutation |
| `orientations` | `Vec<Orientation>` | Per-device orientation |
| `n` | `usize` | Number of devices |
| `rng` | `StdRng` | Seeded RNG (determinism contract) |

### Cost function

```
cost(state) = alpha * area(state)
            + beta  * wirelength(state)
            + gamma * constraint_penalty(state)
```

Where:
- `area(state)` = bounding box area of the decoded placement (in um^2).
- `wirelength(state)` = half-perimeter wirelength (HPWL) summed over all nets.
  Computed from the decoded (x, y) coordinates and device terminal positions.
- `constraint_penalty(state)` = sum of hard-constraint violation penalties.
  Each violated hard constraint adds a large fixed penalty. See
  [05-constraints.md](05-constraints.md) for how each constraint type
  contributes.

The weights `alpha`, `beta`, `gamma` are tuned. Starting values:

```
alpha = 1.0          // area always matters
beta  = 0.5          // wirelength is secondary to area for analog
gamma = 1000.0       // hard constraint violations must dominate
```

During annealing, `gamma` is kept constant (hard constraints never relax).
`alpha` and `beta` can be adjusted per temperature tier if needed.

### Temperature schedule

Geometric cooling:

```
T_0     = initial temperature (calibrated from initial cost variance)
alpha_t = cooling factor = 0.95
T_min   = 1e-4
iters_per_temp = 10 * n  (scale with problem size)
```

Initial temperature calibration:
1. Run 1000 random moves from the initial state
2. Compute the standard deviation of cost changes
3. Set T_0 = 20 * stddev (so ~90% of uphill moves are accepted initially)

### Acceptance criterion

Standard Metropolis:
```
delta = cost(new_state) - cost(current_state)
if delta < 0:
    accept
else:
    accept with probability exp(-delta / T)
```

### Termination

Stop when any of:
- Temperature drops below T_min
- No accepted move in 5 consecutive temperature levels
- Total iterations exceed a budget (default: 500 * n^2)

## Move operators

### Basic moves (always available)

1. **Swap in Gamma+:** Pick two random positions in Gamma+, swap the devices.
   O(1). Changes horizontal/vertical relationships of the two devices with
   all others.

2. **Swap in Gamma-:** Same, in Gamma-. O(1).

3. **Swap in both:** Swap the same two devices in both Gamma+ and Gamma-.
   This specifically changes their left/right to above/below or vice versa.
   O(1).

4. **Rotate:** Pick a random device, change its orientation from the legal
   set. For rotation by 90/270 degrees, swap its width and height. O(1).

### Constraint-aware moves (see [05-constraints.md](05-constraints.md))

5. **Symmetric swap:** For a symmetric pair (A, A'), swap them jointly in
   both sequences to maintain mirror symmetry. O(1).

6. **Symmetric mirror:** For a symmetric pair, flip A's orientation to MY(A)
   and A' to MY(A'). Maintains the symmetry invariant. O(1).

7. **Group move:** For an alignment or order group, shift all members
   together by swapping them as a block in one sequence. O(k) where k is
   group size.

### Move selection probability

| Move | Probability | Rationale |
|------|------------|-----------|
| Swap Gamma+ | 0.25 | Core exploration |
| Swap Gamma- | 0.25 | Core exploration |
| Swap both | 0.10 | Topology change |
| Rotate | 0.10 | Orientation exploration |
| Symmetric swap | 0.15 | Constraint-preserving (if constraints exist) |
| Symmetric mirror | 0.05 | Constraint-preserving |
| Group move | 0.10 | Constraint-preserving |

If no symmetric/group constraints exist, their probability is redistributed
to the basic moves.

## HPWL computation

For each net, compute the half-perimeter of the bounding box of its terminal
positions:

```
HPWL(net) = (max_x - min_x) + (max_y - min_y)
```

Where for each pin `p` on net `net`:
```
pin_x = device_x[p.device] + terminal_offset_x[p.terminal]
pin_y = device_y[p.device] + terminal_offset_y[p.terminal]
```

Terminal offsets come from the primitive's terminal map, adjusted for the
device's current orientation.

Total wirelength = sum of HPWL over all nets.

For efficiency, maintain an incremental HPWL data structure: after a move that
affects devices {A, B}, only recompute HPWL for nets connected to A or B.
This requires a net-to-device adjacency structure from the netlist.

## Initial state generation

1. **Order devices** by area (largest first) for a better initial packing.
2. **Set Gamma+ = Gamma-** = this sorted order (produces a diagonal/staircase
   packing — a valid starting point).
3. **Set all orientations** to R0.
4. **Decode** to get initial (x, y) coordinates.
5. **Apply constraint initialization:** for symmetric pairs, set their
   positions in Gamma+/Gamma- to the mirror-feasible configuration
   (see [05-constraints.md](05-constraints.md)).

## Determinism

The SA loop is deterministic given a seed:
- `StdRng::seed_from_u64(seed)` for the RNG
- Device ordering is canonical (sorted by name, breaking ties by index)
- All tie-breaking uses the canonical order, never HashMap iteration order
- The seed is recorded in `PlacementCertificate::seed`

Two runs with the same input and seed produce byte-identical output.

## Output

After SA terminates, the final state is:
- `gamma_plus`, `gamma_minus`, `orientations` — the best sequence-pair found
- Decoded `(x, y)` coordinates for each device
- Cost breakdown (area, wirelength, constraint penalty)
- `iterations` count, `explored_nodes` (total moves evaluated)

This is passed to the legalization phase ([07-legalization.md](07-legalization.md))
and then to the gate evaluation oracles.
