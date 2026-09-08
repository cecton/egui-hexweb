//! Exact counting of the puzzle's solutions.
//!
//! A **solution** is a placement of the multiset of pieces on distinct nodes
//! such that every arrow of every piece points at an occupied node. Pieces
//! with identical arrows are interchangeable: two placements that differ only
//! by permuting them are the same solution. Every count in this module is
//! stated in those terms; don't "optimize" the grouping away, it is the
//! property the uniqueness guarantee is stated in.
//!
//! The boards this crate ships are small (8-16 nodes, 6-12 pieces), which
//! makes an exhaustive search cheap and exact where a heuristic would need
//! caveats:
//!
//! 1. Enumerate the *empty* sets instead of the occupied ones (the smaller
//!    side: C(16,4) = 1820 at worst). Deciding occupancy first turns the
//!    self-referential "arrows must point at pieces" constraint into a purely
//!    local check: a piece with arrow set `a` fits on node `n` iff every
//!    arrow target of `n` is in-board and occupied.
//! 2. Per occupancy set, assign pieces group by group (grouped by arrow set,
//!    canonical bitmask order). A group of `c` identical pieces picks `c`
//!    fitting nodes in **increasing node order**, which collapses all `c!`
//!    permutations to one count.
//! 3. Prune whenever a group has fewer fitting nodes than copies, and stop
//!    as soon as the count reaches the caller's cap.
//!
//! Cost at the shipped sizes: a few thousand occupancy sets times a shallow
//! DFS — tens of microseconds, so the generator verifies every candidate and
//! every repair step.

use crate::game::{Arrows, Node, Piece};

/// Counts the distinct solutions, stopping early once `cap` is reached (the
/// result is then `cap`). `cap == 0` always returns 0.
pub(crate) fn count(pieces: &[Piece], nodes: &[Node], cap: usize) -> usize {
    let mut search = Search::new(cap, false);
    if cap > 0 {
        run(pieces, nodes, &mut search);
    }
    search.count
}

/// One solving placement: the node each piece occupies, indexed by piece id.
/// `None` if the board has no solution.
pub(crate) fn solution(pieces: &[Piece], nodes: &[Node]) -> Option<Vec<NodeId>> {
    let mut search = Search::new(1, true);
    run(pieces, nodes, &mut search);
    let slot_piece = search.slot_piece;
    search.solution.map(|slots| {
        let mut per_piece = vec![0; slot_piece.len()];
        for (slot, &node) in slots.iter().enumerate() {
            per_piece[slot_piece[slot]] = node;
        }
        per_piece
    })
}

/// One solving placement other than `exclude` (a node-per-piece placement):
/// the node each piece occupies, indexed by piece id. `None` if the board
/// has no solution other than `exclude`.
///
/// The generator uses this with its constructed configuration as `exclude`:
/// `None` there means the configuration is the *only* solution. Solutions
/// are compared in canonical (group-major) form, so "equal" means the same
/// placement of the multiset, not a permutation of identical pieces.
pub(crate) fn alternate_solution(
    pieces: &[Piece],
    nodes: &[Node],
    exclude: &[NodeId],
) -> Option<Vec<NodeId>> {
    let mut search = Search::new(usize::MAX, true);
    // Reorder the excluded placement into slot (group-major) order to match
    // the search's slot representation.
    let mut order: Vec<usize> = (0..pieces.len()).collect();
    order.sort_unstable_by_key(|&piece| pieces[piece].arrows.bits());
    let mut exclude_slots = vec![0; pieces.len()];
    for (slot, &piece) in order.iter().enumerate() {
        exclude_slots[slot] = exclude[piece];
    }
    search.exclude = Some(exclude_slots);
    run(pieces, nodes, &mut search);
    let slot_piece = search.slot_piece;
    search.solution.map(|slots| {
        let mut per_piece = vec![0; slot_piece.len()];
        for (slot, &node) in slots.iter().enumerate() {
            per_piece[slot_piece[slot]] = node;
        }
        per_piece
    })
}

type NodeId = usize;

struct Search {
    cap: usize,
    count: usize,
    want_solution: bool,
    /// Nodes per slot (group-major piece order); only kept when
    /// `want_solution`.
    solution: Option<Vec<NodeId>>,
    /// Which piece id each slot belongs to.
    slot_piece: Vec<usize>,
    /// A solution (in slot order) to skip, for
    /// [`alternate_solution`].
    exclude: Option<Vec<NodeId>>,
}

impl Search {
    fn new(cap: usize, want_solution: bool) -> Self {
        Self {
            cap,
            count: 0,
            want_solution,
            solution: None,
            slot_piece: Vec::new(),
            exclude: None,
        }
    }

    /// Records a found solution. Returns whether the search is saturated.
    fn found(&mut self, slots: &[NodeId]) -> bool {
        if self
            .exclude
            .as_ref()
            .is_some_and(|exclude| slots == exclude)
        {
            return false; // skip the excluded solution, keep searching
        }
        self.count += 1;
        if self.want_solution && self.solution.is_none() {
            self.solution = Some(slots.to_vec());
        }
        self.count >= self.cap
    }
}

fn run(pieces: &[Piece], nodes: &[Node], search: &mut Search) {
    // Group pieces by arrow set, in the canonical (bitmask) order, and
    // remember which piece id each slot belongs to so `solution` can report
    // nodes per piece.
    let mut order: Vec<usize> = (0..pieces.len()).collect();
    order.sort_unstable_by_key(|&piece| pieces[piece].arrows.bits());
    search.slot_piece = order.clone();
    let mut groups: Vec<(Arrows, usize)> = Vec::new();
    for &piece in &order {
        let arrows = pieces[piece].arrows;
        match groups.last_mut() {
            Some((last, count)) if *last == arrows => *count += 1,
            _ => groups.push((arrows, 1)),
        }
    }

    let piece_count = pieces.len();
    let node_count = nodes.len();
    debug_assert!(node_count < 64, "occupancy masks are u64");
    if piece_count > node_count {
        return; // impossible placement: no solution
    }
    if piece_count == 0 {
        search.found(&[]);
        return;
    }

    let empties = node_count - piece_count;
    let full: u64 = (1u64 << node_count) - 1;

    // Per group and node, the combined bitmask of the node's arrow targets:
    // `None` when any target is off the board (the group can never sit
    // there), `Some(mask)` otherwise (the group fits wherever every target
    // bit is occupied; an empty arrow set demands nothing). Precomputing
    // this turns the per-occupancy-set work into a handful of ANDs per
    // node, which is what makes exhaustive counting cheap enough to run on
    // every generated candidate.
    let group_masks: Vec<Vec<Option<u64>>> = groups
        .iter()
        .map(|&(arrows, _)| {
            (0..node_count)
                .map(|id| {
                    let mut mask = 0u64;
                    for dir in arrows.iter() {
                        match nodes[id].neighbors[dir as usize] {
                            Some(neighbor) => mask |= 1 << neighbor,
                            None => return None,
                        }
                    }
                    Some(mask)
                })
                .collect()
        })
        .collect();

    // Enumerate empty sets from the smaller side (Gosper's hack over
    // popcount-`empties` masks); the occupied set is the complement. The
    // per-set fit lists live in reused buffers: a fresh Vec per occupancy
    // set allocates tens of thousands of times per sweep.
    let mut slots: Vec<NodeId> = Vec::with_capacity(piece_count);
    let mut fits: Vec<Vec<NodeId>> = vec![Vec::with_capacity(node_count); groups.len()];
    for empty in EmptySets::new(empties, node_count) {
        let occupied = full & !empty;

        // Per group, the fitting nodes for this occupancy set. Prefilter:
        // skip the set entirely if any group cannot be seated.
        let mut seatable = true;
        for (gi, &(_, copies)) in groups.iter().enumerate() {
            let masks = &group_masks[gi];
            let fitting = &mut fits[gi];
            fitting.clear();
            fitting.extend((0..node_count).filter(|&id| {
                occupied >> id & 1 == 1
                    && matches!(masks[id], Some(mask) if mask & occupied == mask)
            }));
            if fitting.len() < copies {
                seatable = false;
                break;
            }
        }
        if !seatable {
            continue;
        }

        // Candidates are already filtered to `occupied`, so the DFS's used
        // mask starts empty.
        let seating = Seating {
            groups: &groups,
            fits: &fits,
        };
        if seating.groups(0, 0, &mut slots, search) {
            return; // cap reached
        }
    }
}

/// The per-occupancy-set DFS: seats each group's copies on fitting nodes.
/// `groups` and `fits` always travel together through the recursion, so
/// they live on the struct rather than threading through every level.
struct Seating<'a> {
    groups: &'a [(Arrows, usize)],
    fits: &'a [Vec<NodeId>],
}

impl Seating<'_> {
    /// Assigns groups starting at `gi`. Returns whether the search is
    /// saturated.
    fn groups(&self, gi: usize, used: u64, slots: &mut Vec<NodeId>, search: &mut Search) -> bool {
        if gi == self.groups.len() {
            return search.found(slots);
        }
        let (_, copies) = self.groups[gi];
        self.copies(gi, copies, 0, used, slots, search)
    }

    /// Seats the remaining `copies_left` copies of group `gi`, starting at
    /// candidate index `start`. The strictly increasing index is what makes
    /// identical pieces' permutations count once: they are seated in node
    /// order.
    fn copies(
        &self,
        gi: usize,
        copies_left: usize,
        start: usize,
        used: u64,
        slots: &mut Vec<NodeId>,
        search: &mut Search,
    ) -> bool {
        if copies_left == 0 {
            return self.groups(gi + 1, used, slots, search);
        }
        for (idx, &node) in self.fits[gi].iter().enumerate().skip(start) {
            if used >> node & 1 == 1 {
                continue;
            }
            slots.push(node);
            if self.copies(
                gi,
                copies_left - 1,
                idx + 1,
                used | 1 << node,
                slots,
                search,
            ) {
                return true;
            }
            slots.pop();
        }
        false
    }
}

/// Successor masks of fixed popcount in increasing order (Gosper's hack).
struct EmptySets {
    mask: u64,
    limit: u64,
    done: bool,
}

impl EmptySets {
    fn new(empties: usize, node_count: usize) -> Self {
        if empties == 0 {
            return Self {
                mask: 0,
                limit: 0,
                done: false,
            };
        }
        Self {
            mask: (1 << empties) - 1,
            limit: 1 << node_count,
            done: false,
        }
    }
}

impl Iterator for EmptySets {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        if self.done {
            return None;
        }
        let result = self.mask;
        // Special case `empties == 0`: exactly one (empty) set.
        if self.mask == 0 {
            self.done = true;
            return Some(result);
        }
        let smallest = self.mask & self.mask.wrapping_neg();
        let incremented = self.mask + smallest;
        if incremented == 0 || incremented >= self.limit {
            self.done = true;
            return Some(result);
        }
        self.mask = (((self.mask ^ incremented) / smallest) >> 2) | incremented;
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{build_nodes, Dir};

    fn game_count(arrows: &[Arrows], coords: &[(i32, i32)], cap: usize) -> usize {
        let (nodes, _) = build_nodes(coords);
        let pieces: Vec<Piece> = arrows.iter().map(|&arrows| Piece { arrows }).collect();
        count(&pieces, &nodes, cap)
    }

    /// Independent, deliberately naive reference: enumerate every placement
    /// of *labeled* pieces, keep the ones where all arrows are satisfied,
    /// canonicalize each to (coordinate, arrows) pairs sorted by coordinate,
    /// and count distinct canonical forms. Slow, and shares no logic with
    /// the solver.
    fn naive_count(arrows: &[Arrows], coords: &[(i32, i32)]) -> usize {
        let (nodes, _) = build_nodes(coords);
        let mut solutions = std::collections::HashSet::new();
        for placement in Placements::new(arrows.len(), coords.len()) {
            let mut occupied = vec![false; coords.len()];
            for &node in &placement {
                occupied[node] = true;
            }
            let mut satisfied = true;
            'pieces: for (&node, &arrows) in placement.iter().zip(arrows) {
                for dir in arrows.iter() {
                    match nodes[node].neighbors[dir as usize] {
                        Some(neighbor) if occupied[neighbor] => {}
                        _ => {
                            satisfied = false;
                            break 'pieces;
                        }
                    }
                }
            }
            if satisfied {
                let mut canonical: Vec<(NodeId, u8)> = placement
                    .iter()
                    .zip(arrows)
                    .map(|(&node, &arrows)| (node, arrows.bits()))
                    .collect();
                canonical.sort_unstable();
                let canonical: Vec<(i32, i32, u8)> = canonical
                    .into_iter()
                    .map(|(node, bits)| (nodes[node].q, nodes[node].r, bits))
                    .collect();
                solutions.insert(canonical);
            }
        }
        solutions.len()
    }

    /// All placements of `k` distinct pieces on `n` nodes, as node lists.
    /// Yields the current state first, then advances: the last combination
    /// must not be swallowed by the final wrap.
    struct Placements {
        indices: Vec<usize>,
        k: usize,
        n: usize,
        first: bool,
        done: bool,
    }

    impl Placements {
        fn new(k: usize, n: usize) -> Self {
            Self {
                indices: (0..k).collect(),
                k,
                n,
                first: k <= n,
                done: k > n,
            }
        }
    }

    impl Iterator for Placements {
        type Item = Vec<usize>;

        fn next(&mut self) -> Option<Vec<usize>> {
            if self.done {
                return None;
            }
            if self.first {
                self.first = false;
            } else {
                // Advance the odometer to the next distinct-indices state.
                loop {
                    let mut i = self.k;
                    loop {
                        if i == 0 {
                            // Wrapped: every state has been yielded.
                            self.done = true;
                            return None;
                        }
                        i -= 1;
                        self.indices[i] += 1;
                        if self.indices[i] == self.n {
                            self.indices[i] = 0;
                        } else {
                            break;
                        }
                    }
                    let mut sorted = self.indices.clone();
                    sorted.sort_unstable();
                    sorted.dedup();
                    if sorted.len() == self.k {
                        break;
                    }
                }
            }
            Some(self.indices.clone())
        }
    }

    /// The 3-node vertical line: nodes 0=(0,0), 1=(0,-1), 2=(0,1).
    fn line() -> Vec<(i32, i32)> {
        vec![(0, 0), (0, -1), (0, 1)]
    }

    #[test]
    fn two_opposed_arrows_have_two_solutions() {
        let up = Arrows::from_dir(Dir::Up);
        let down = Arrows::from_dir(Dir::Down);
        // Occupied {node 0, node 1} and {node 0, node 2} both work.
        assert_eq!(game_count(&[up, down], &line(), 10), 2);
        // The cap saturates the count.
        assert_eq!(game_count(&[up, down], &line(), 1), 1);
        assert_eq!(game_count(&[up, down], &line(), 0), 0);
    }

    #[test]
    fn unsatisfiable_boards_count_zero() {
        let up = Arrows::from_dir(Dir::Up);
        // A single piece has nothing to point at: any arrow needs another
        // piece on its target node.
        assert_eq!(game_count(&[up], &line(), 10), 0);
        // Two Up pieces on a line: the topmost one always points off-board.
        assert_eq!(game_count(&[up, up], &line(), 10), 0);
    }

    #[test]
    fn identical_pieces_are_not_double_counted() {
        // An all-identical multiset can never be satisfied at all: if every
        // piece carries arrow d, piece at x demands another piece at x+d,
        // forever. Identical pieces only work inside a mixed multiset:
        // [Up, Up, Down] on the 3-line has exactly one solution (Down at
        // (0,-1), the two Ups stacked above it), where the naive labeled
        // counter would see the two Ups' 2! permutations before
        // canonicalizing.
        let up = Arrows::from_dir(Dir::Up);
        let down = Arrows::from_dir(Dir::Down);
        let count = game_count(&[up, up, down], &line(), usize::MAX);
        assert_eq!(count, naive_count(&[up, up, down], &line()));
        assert_eq!(count, 1);
    }

    #[test]
    fn solution_is_actually_a_solution() {
        let up = Arrows::from_dir(Dir::Up);
        let down = Arrows::from_dir(Dir::Down);
        let (nodes, _) = build_nodes(&line());
        let pieces = vec![Piece { arrows: up }, Piece { arrows: down }];
        let found = solution(&pieces, &nodes).expect("has solutions");
        assert_eq!(found.len(), 2);
        // Re-check independently: both arrows point at an occupied node.
        let occupied: std::collections::HashSet<NodeId> = found.iter().copied().collect();
        for (&node, piece) in found.iter().zip(&pieces) {
            for dir in piece.arrows.iter() {
                let neighbor = nodes[node].neighbors[dir as usize]
                    .expect("line board has no off-board solutions here");
                assert!(occupied.contains(&neighbor));
            }
        }
    }

    #[test]
    fn exhaustive_brute_force_cross_check() {
        let mut rng = fastrand::Rng::with_seed(0xC0FFEE);
        for case in 0..300 {
            // Small random boards: a random subset of the 7-node hexagon,
            // 2-4 pieces with 1-3 arrows each, drawn from a tiny pool so
            // identical pieces (the grouping edge case) occur constantly.
            let mut coords = crate::game::hexagon(1);
            let keep = rng.usize(3..=coords.len());
            for _ in 0..coords.len() - keep {
                coords.remove(rng.usize(..coords.len()));
            }
            let pool = [
                Arrows::from_dir(Dir::Up),
                Arrows::from_dir(Dir::Down),
                Arrows::from_dir(Dir::Up).with(Dir::Down),
            ];
            let piece_count = rng.usize(2..=4.min(coords.len() - 1));
            let arrows: Vec<Arrows> = (0..piece_count)
                .map(|_| {
                    let mut arrows = pool[rng.usize(..pool.len())];
                    // Maybe bolt on one extra direction for variety.
                    if rng.bool() {
                        arrows = arrows.with(Dir::ALL[rng.usize(..6)]);
                    }
                    arrows
                })
                .collect();

            let expected = naive_count(&arrows, &coords);
            let got = game_count(&arrows, &coords, usize::MAX);
            assert_eq!(
                got, expected,
                "case {case}: solver {got} vs naive {expected} for {arrows:?} on {coords:?}"
            );
        }
    }

    #[test]
    fn empty_sets_iterator_enumerates_exactly_combinations() {
        let combinations = |n: usize, e: usize| {
            let numer: u64 = ((n - e + 1)..=n).map(|v| v as u64).product();
            let denom: u64 = (1..=e).map(|v| v as u64).product();
            numer / denom
        };
        for node_count in 1..=12usize {
            for empties in 0..=node_count {
                let seen: Vec<u64> = EmptySets::new(empties, node_count).collect();
                assert_eq!(
                    seen.len() as u64,
                    combinations(node_count, empties),
                    "n={node_count} e={empties}"
                );
                assert!(seen.iter().all(|&m| m.count_ones() == empties as u32));
                assert_eq!(
                    seen.iter().collect::<std::collections::HashSet<_>>().len(),
                    seen.len(),
                    "duplicates for n={node_count} e={empties}"
                );
            }
        }
    }
}
