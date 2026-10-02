# Mutual Arrows Rule Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An arrow is satisfied only when the piece it points at points back with the opposite arrow; solver, generator, feedback, and docs all enforce the stricter rule.

**Architecture:** Keep every existing algorithm skeleton. `unsatisfied_arrows` gains a reciprocity step after its occupied check; the solver adds a pairwise mutual check while seating groups (tracked via a `group_at` buffer); the generator closes its constructed solution under reciprocity and repairs with arrow *pairs*, preserving the monotone-repair argument.

**Tech Stack:** Rust, no new dependencies. Test with the crate's own suite (`cargo test --lib`), clippy, fmt.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-10-02-mutual-arrows-design.md`.
- Never use the names "Hexa Arrows" or "Hexcells" anywhere.
- Multiset grouping in the solver stays exactly as is (groups pick nodes in increasing order).
- No losing state, no timer, latched win: unchanged.
- Seeded, reproducible generation: unchanged; `fastrand` only.
- Board sizes stay 8-16 nodes; occupancy masks stay `u64`.
- Check suite per task: `cargo check && cargo test --lib && cargo clippy -- -D warnings && cargo fmt --check` (run from repo root).
- AGENTS.md rule sentence is updated in Task 4 (docs) so the repo description matches the code by the end.

**Rule (canonical statement, used everywhere):** piece `p` on node `n` with arrow `d` is satisfied iff `nodes[n].neighbors[d]` is in-board and occupied **and** the piece `q` on that neighbor has `q.arrows.contains(d.opposite())`. Solved = every arrow of every piece satisfied.

---

### Task 1: Generator — mutual constructed solutions, paired repairs

**Files:**
- Modify: `src/generator.rs` (functions `assign_arrows` ~line 148, `repair_until_unique` ~line 192, `is_solved_placement` ~line 334; module doc lines 1-33; tests module)
- Test: `src/generator.rs` tests module (new test `assigned_arrows_are_mutual_within_the_solution`)

**Interfaces:**
- Consumes: `crate::game::{Arrows, Dir, Node, NodeId, Piece}` (unchanged), `crate::solver::{alternate_solution, count}` (unchanged signatures).
- Produces: `generate()` now returns boards whose constructed configuration is mutual; `Generated` and all `pub(crate)` signatures unchanged. Task 2 and 3 rely on: the constructed configuration is a solution under the *strict* rule, so the strict solver's count can never reach 0.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `src/generator.rs`. Extend the existing use line to `use crate::game::{build_nodes, random_symmetric_board, HexwebGame, Params};`:

```rust
    #[test]
    fn assigned_arrows_are_mutual_within_the_solution() {
        for seed in 0..10u64 {
            let coords = random_symmetric_board(12, seed);
            let (nodes, _) = build_nodes(&coords);
            let mut rng = fastrand::Rng::with_seed(seed);
            let solution_nodes = pick_occupancy(&nodes, 9, &mut rng).expect("occupancy found");
            let arrows = assign_arrows(&nodes, &solution_nodes, 2, 4, &mut rng);
            for (piece, &node) in solution_nodes.iter().enumerate() {
                for dir in arrows[piece].iter() {
                    let neighbor = nodes[node].neighbors[dir as usize]
                        .expect("assigned arrows point in-board");
                    let other = solution_nodes
                        .iter()
                        .position(|&n| n == neighbor)
                        .expect("assigned arrows point at occupied nodes");
                    assert!(
                        arrows[other].contains(dir.opposite()),
                        "arrow {dir:?} of piece {piece} is not reciprocated (seed {seed})"
                    );
                }
            }
        }
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib assigned_arrows_are_mutual -- --nocapture`
Expected: FAIL — random draws are not reciprocated.

- [ ] **Step 3: Implement the closure pass in `assign_arrows`**

Replace the whole `assign_arrows` function with (the per-piece draw is unchanged; the `Vec` destination and the appended closure loop are new):

```rust
fn assign_arrows(
    nodes: &[Node],
    solution_nodes: &[NodeId],
    min_arrows: usize,
    max_arrows: usize,
    rng: &mut fastrand::Rng,
) -> Vec<Arrows> {
    let mut arrows: Vec<Arrows> = solution_nodes
        .iter()
        .map(|&node| {
            let available: Vec<Dir> = Dir::ALL
                .into_iter()
                .filter(|&dir| {
                    nodes[node].neighbors[dir as usize]
                        .is_some_and(|neighbor| solution_nodes.contains(&neighbor))
                })
                .collect();
            debug_assert!(
                !available.is_empty(),
                "occupancy picker only picks supported nodes"
            );
            let want = rng.usize(min_arrows..=max_arrows).clamp(1, available.len());
            // Partial shuffle: `want` distinct directions.
            let mut available = available;
            let mut arrows = Arrows::default();
            for i in 0..want {
                let j = rng.usize(i..available.len());
                available.swap(i, j);
                arrows = arrows.with(available[i]);
            }
            arrows
        })
        .collect();

    // Close the arrow sets under reciprocity: every arrow must be met by
    // the opposite arrow on its target piece. One pass suffices: an arrow
    // added here points back at a piece that already points this way, so
    // it can never create a new missing reciprocal. This may push a piece
    // past `max_arrows`, which is a soft cap by design.
    for (piece, &node) in solution_nodes.iter().enumerate() {
        for dir in arrows[piece].iter().collect::<Vec<Dir>>() {
            let target = nodes[node].neighbors[dir as usize]
                .expect("assigned arrows only point in-board");
            let other = solution_nodes
                .iter()
                .position(|&n| n == target)
                .expect("assigned arrows only point at occupied nodes");
            let opposite = dir.opposite();
            if !arrows[other].contains(opposite) {
                arrows[other] = arrows[other].with(opposite);
            }
        }
    }
    arrows
}
```

(`Dir` and `Arrows` are already imported.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib assigned_arrows_are_mutual`
Expected: PASS.

- [ ] **Step 5: Make repairs add arrow pairs**

In `repair_until_unique`, replace the kill-classification and arrow-addition inside the `for piece in 0..arrows.len()` loop (from `let mut killers` through the `if !pool.is_empty()` block) with:

```rust
            let mut killers: Vec<Dir> = Vec::new();
            let mut others: Vec<Dir> = Vec::new();
            for dir in Dir::ALL {
                if arrows[piece].contains(dir) {
                    continue;
                }
                // Only arrows that keep the constructed solution valid.
                let Some(target) = nodes[solution_nodes[piece]].neighbors[dir as usize] else {
                    continue;
                };
                if !solution_nodes.contains(&target) {
                    continue;
                }
                let other = solution_nodes
                    .iter()
                    .position(|&n| n == target)
                    .expect("target is in solution_nodes");
                let opposite = dir.opposite();
                // The alternate dies if either half of the added pair is
                // unsatisfied on the alternate placement.
                let kills = [(alternate[piece], dir), (alternate[other], opposite)]
                    .iter()
                    .any(|&(from, d)| match nodes[from].neighbors[d as usize] {
                        Some(neighbor) => alt_occupied >> neighbor & 1 == 0,
                        None => true,
                    });
                if kills {
                    killers.push(dir);
                } else {
                    others.push(dir);
                }
            }
            let pool = if killers.is_empty() {
                &others
            } else {
                &killers
            };
            if !pool.is_empty() {
                let dir = pool[rng.usize(..pool.len())];
                arrows[piece] = arrows[piece].with(dir);
                // Keep the constructed configuration mutual: the target
                // piece must point back. Both new arrows are satisfied
                // within the constructed configuration, and arrows are only
                // ever added, so the repair stays monotone.
                let target = nodes[solution_nodes[piece]].neighbors[dir as usize]
                    .expect("repair arrows only point in-board");
                let other = solution_nodes
                    .iter()
                    .position(|&n| n == target)
                    .expect("repair arrows only point at occupied nodes");
                let opposite = dir.opposite();
                if !arrows[other].contains(opposite) {
                    arrows[other] = arrows[other].with(opposite);
                }
                repairs += 1;
                added += 1;
            }
```

- [ ] **Step 6: Make `is_solved_placement` require reciprocity**

Replace the whole function:

```rust
fn is_solved_placement(nodes: &[Node], arrows: &[Arrows], cell: &[Option<PieceId>]) -> bool {
    cell.iter().enumerate().all(|(node, &occupant)| {
        let Some(piece) = occupant else {
            return true;
        };
        arrows[piece].iter().all(|dir| match nodes[node].neighbors[dir as usize] {
            Some(neighbor) => match cell[neighbor] {
                Some(other) => arrows[other].contains(dir.opposite()),
                None => false,
            },
            None => false,
        })
    })
}
```

- [ ] **Step 7: Update the module doc pipeline**

In the module doc (`src/generator.rs` top), amend step 2 to end with: "A closing pass then adds every missing reciprocal arrow, so the constructed configuration is a web of mutual pairs — a solution *by construction* under the mutual rule (a piece may end up above `max_arrows`, which is a soft cap)." Amend step 3's monotone sentence to: "If there is one, add an arrow *pair*: the new arrow plus the opposite arrow on its target piece. Adding a pair is **monotone**: both arrows are mutual within the constructed configuration (so it stays valid and the count can never reach zero), and a strictly stronger constraint can never *create* a solution (so the count can never increase)."

- [ ] **Step 8: Run the full generator test module + suite**

Run: `cargo test --lib && cargo clippy -- -D warnings && cargo fmt --check`
Expected: all PASS. (Solver and game are still lenient here: generated boards are unique under the stricter rule too, and scramble checks only got stricter, so nothing else breaks.)

- [ ] **Step 9: Commit**

```bash
rtk git add src/generator.rs
rtk git commit -m "feat(generator): build constructed solutions from mutual arrow pairs"
```

---

### Task 2: Solver — pairwise mutual check during seating

**Files:**
- Modify: `src/solver.rs` (module doc lines 1-28; struct `Seating` ~line 235; methods `groups`/`copies` ~line 243-285; function `run` ~line 136-230 where `Seating` is constructed; tests module)
- Test: `src/solver.rs` tests module

**Interfaces:**
- Consumes: `crate::game::{Arrows, Dir, Node, Piece}` unchanged; `Dir::opposite()` and `Arrows::{contains, bits, iter}` existing.
- Produces: `count`, `solution`, `alternate_solution` with unchanged signatures, now counting only fully mutual placements. Task 3's win check and Task 1's uniqueness verification both rely on this.

- [ ] **Step 1: Update the naive reference to the mutual rule**

In `naive_count`, replace the satisfaction loop:

```rust
            let mut satisfied = true;
            'pieces: for (&node, &piece_arrows) in placement.iter().zip(arrows) {
                for dir in piece_arrows.iter() {
                    let ok = match nodes[node].neighbors[dir as usize] {
                        Some(neighbor) if occupied[neighbor] => {
                            let other = placement
                                .iter()
                                .position(|&n| n == neighbor)
                                .expect("occupied neighbor holds a piece");
                            arrows[other].contains(dir.opposite())
                        }
                        _ => false,
                    };
                    if !ok {
                        satisfied = false;
                        break 'pieces;
                    }
                }
            }
```

Also update the module doc (lines 3-4) to: "A **solution** is a placement of the multiset of pieces on distinct nodes such that every arrow of every piece points at an occupied node **whose piece points back with the opposite arrow**." and step 1's local-check sentence to mention that reciprocity between already-seated neighbors is checked during seating (see below).

- [ ] **Step 2: Update the counting tests**

Replace `identical_pieces_are_not_double_counted` and extend `unsatisfiable_boards_count_zero`:

```rust
    #[test]
    fn unsatisfiable_boards_count_zero() {
        let up = Arrows::from_dir(Dir::Up);
        // A single piece has nothing to point at: any arrow needs another
        // piece on its target node.
        assert_eq!(game_count(&[up], &line(), 10), 0);
        // Two Up pieces on a line: the topmost one always points off-board.
        assert_eq!(game_count(&[up, up], &line(), 10), 0);
        // Under the mutual rule this old fixture dies too: the two Up
        // pieces can never be pointed back at.
        let down = Arrows::from_dir(Dir::Down);
        assert_eq!(game_count(&[up, up, down], &line(), 10), 0);
    }

    #[test]
    fn identical_pieces_are_not_double_counted() {
        // A full 4-node line where every arrow is mutual, top to bottom:
        // {Down}, {Up, Down}, {Up, Down}, {Up}. The two identical
        // {Up, Down} pieces are seated in increasing node order, so their
        // 2! permutations count once.
        let up = Arrows::from_dir(Dir::Up);
        let down = Arrows::from_dir(Dir::Down);
        let both = up.with(Dir::Down);
        let coords = [(0, 0), (0, -1), (0, 1), (0, 2)];
        let arrows = [down, both, both, up];
        let count = game_count(&arrows, &coords, usize::MAX);
        assert_eq!(count, naive_count(&arrows, &coords));
        assert_eq!(count, 1);
    }
```

In `solution_is_actually_a_solution`, extend the re-check loop with reciprocity:

```rust
        // Re-check independently: both arrows point at an occupied node,
        // and that node's piece points back.
        let occupied: std::collections::HashSet<NodeId> = found.iter().copied().collect();
        for (&node, piece) in found.iter().zip(&pieces) {
            for dir in piece.arrows.iter() {
                let neighbor = nodes[node].neighbors[dir as usize]
                    .expect("line board has no off-board solutions here");
                assert!(occupied.contains(&neighbor));
                let other = found.iter().position(|&n| n == neighbor).unwrap();
                assert!(pieces[other].arrows.contains(dir.opposite()));
            }
        }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --lib solver`
Expected: FAIL — the real solver (lenient) counts more than the naive reference (mutual): `identical_pieces_are_not_double_counted` and `exhaustive_brute_force_cross_check` mismatch.

- [ ] **Step 4: Implement the pairwise check**

In `run`, after `fits` is built, add the buffer and pass it to `Seating`:

```rust
    let mut slots: Vec<NodeId> = Vec::with_capacity(piece_count);
    let mut fits: Vec<Vec<NodeId>> = vec![Vec::with_capacity(node_count); groups.len()];
    // Which group is seated on each node during the DFS; `None` = unseated.
    let mut group_at: Vec<Option<usize>> = vec![None; node_count];
```

and replace the construction/call at the bottom of the loop:

```rust
        let mut seating = Seating {
            nodes,
            groups: &groups,
            fits: &fits,
            group_at: &mut group_at,
        };
        if seating.groups(0, 0, &mut slots, search) {
            return; // cap reached
        }
```

Replace struct `Seating` and its methods wholesale:

```rust
/// The per-occupancy-set DFS: seats each group's copies on fitting nodes.
/// `groups`, `fits`, and `group_at` always travel together through the
/// recursion, so they live on the struct rather than threading through
/// every level.
struct Seating<'a> {
    nodes: &'a [Node],
    groups: &'a [(Arrows, usize)],
    fits: &'a [Vec<NodeId>],
    /// Which group is seated on each node, maintained with `slots`.
    group_at: &'a mut Vec<Option<usize>>,
}

impl Seating<'_> {
    /// Assigns groups starting at `gi`. Returns whether the search is
    /// saturated.
    fn groups(&mut self, gi: usize, used: u64, slots: &mut Vec<NodeId>, search: &mut Search) -> bool {
        if gi == self.groups.len() {
            return search.found(slots);
        }
        let (arrows, copies) = self.groups[gi];
        self.copies(gi, arrows, copies, 0, used, slots, search)
    }

    /// Seats the remaining `copies_left` copies of group `gi` (arrow set
    /// `arrows`), starting at candidate index `start`. The strictly
    /// increasing index is what makes identical pieces' permutations count
    /// once: they are seated in node order.
    fn copies(
        &mut self,
        gi: usize,
        arrows: Arrows,
        copies_left: usize,
        start: usize,
        used: u64,
        slots: &mut Vec<NodeId>,
        search: &mut Search,
    ) -> bool {
        if copies_left == 0 {
            return self.groups(gi + 1, used, slots, search);
        }
        for idx in start..self.fits[gi].len() {
            let node = self.fits[gi][idx];
            if used >> node & 1 == 1 {
                continue;
            }
            if !self.mutual_with_seated(node, arrows, gi) {
                continue;
            }
            slots.push(node);
            self.group_at[node] = Some(gi);
            if self.copies(gi, arrows, copies_left - 1, idx + 1, used | 1 << node, slots, search) {
                return true;
            }
            self.group_at[node] = None;
            slots.pop();
        }
        false
    }

    /// Whether seating an `arrows` piece on `node` keeps every adjacent
    /// already-seated pair mutual: either both halves point at each other,
    /// or neither does. Each adjacent pair is checked exactly once — when
    /// its second endpoint is seated — so any completed seating is fully
    /// mutual, and bad branches prune before the leaf.
    fn mutual_with_seated(&self, node: NodeId, arrows: Arrows, gi: usize) -> bool {
        for dir in Dir::ALL {
            let Some(neighbor) = self.nodes[node].neighbors[dir as usize] else {
                continue;
            };
            let Some(other) = self.group_at[neighbor] else {
                continue;
            };
            if arrows.contains(dir) != self.groups[other].0.contains(dir.opposite()) {
                return false;
            }
        }
        true
    }
}
```

(`Dir` needs to be in the solver's imports: change line 30 to `use crate::game::{Arrows, Dir, Node, Piece};`.)

- [ ] **Step 5: Run the solver tests**

Run: `cargo test --lib solver`
Expected: PASS (including the brute-force cross-check against the updated naive reference).

- [ ] **Step 6: Run the full suite**

Run: `cargo test --lib && cargo clippy -- -D warnings && cargo fmt --check`
Expected: PASS. Generated boards from Task 1 are mutual by construction, so the stricter solver still finds their solution (count never 0), and repairs now drive the strict count to 1.

- [ ] **Step 7: Commit**

```bash
rtk git add src/solver.rs
rtk git commit -m "feat(solver): count only placements where every arrow is mutual"
```

---

### Task 3: Game — strict feedback and win check

**Files:**
- Modify: `src/game.rs` (`unsatisfied_arrows` ~line 537; docs of `satisfied` ~531, `is_solved` ~552; module doc lines 1-7; tests module)
- Test: `src/game.rs` tests module

**Interfaces:**
- Consumes: `Dir::opposite()`, `Arrows::{contains, with, iter}` (existing).
- Produces: `unsatisfied_arrows` (same signature) now includes arrows whose target piece does not point back; `is_solved`/`satisfied`/widget feedback derive from it. Task 4's doc text describes this behavior.

- [ ] **Step 1: Write the failing test**

Add to the tests module in `src/game.rs`:

```rust
    #[test]
    fn arrows_without_reciprocal_stay_unsatisfied() {
        // Two {Up} pieces stacked: the lower one points at an occupied
        // node, but the piece there points up, not back down.
        let (nodes, _) = build_nodes(&[(0, 0), (0, -1)]);
        let up = Piece {
            arrows: Arrows::from_dir(Dir::Up),
        };
        let pieces = vec![up, up];
        let cell_piece = vec![Some(0), Some(1)];
        let game = HexwebGame::from_parts(nodes, pieces, cell_piece.clone(), cell_piece);
        assert_eq!(game.unsatisfied_arrows(0), Arrows::from_dir(Dir::Up));
        assert!(!game.is_solved());

        // Making the pair mutual satisfies both arrows.
        let (nodes, _) = build_nodes(&[(0, 0), (0, -1)]);
        let pieces = vec![
            Piece {
                arrows: Arrows::from_dir(Dir::Up),
            },
            Piece {
                arrows: Arrows::from_dir(Dir::Down),
            },
        ];
        let cell_piece = vec![Some(0), Some(1)];
        let game = HexwebGame::from_parts(nodes, pieces, cell_piece.clone(), cell_piece);
        assert_eq!(game.unsatisfied_arrows(0), Arrows::default());
        assert_eq!(game.unsatisfied_arrows(1), Arrows::default());
        assert!(game.is_solved());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib arrows_without_reciprocal`
Expected: FAIL — the second half reports `is_solved() == false` because the win check is still one-way.

- [ ] **Step 3: Implement**

Replace `unsatisfied_arrows` and its two doc comments:

```rust
    /// Whether every arrow of `piece` currently forms a mutual pair.
    pub fn satisfied(&self, piece: PieceId) -> bool {
        self.unsatisfied_arrows(piece).is_empty()
    }

    /// The arrows of `piece` that do not yet form a mutual pair: their
    /// target node is empty, off the board, or holds a piece that does not
    /// point back with the opposite arrow. This is the live feedback the
    /// widget paints in the accent color.
    pub fn unsatisfied_arrows(&self, piece: PieceId) -> Arrows {
        let Some(node) = self.node_of(piece) else {
            return self.pieces[piece].arrows;
        };
        let mut result = Arrows::default();
        for dir in self.pieces[piece].arrows.iter() {
            let satisfied = match self.nodes[node].neighbors[dir as usize] {
                Some(neighbor) => match self.cell_piece[neighbor] {
                    Some(other) => self.pieces[other].arrows.contains(dir.opposite()),
                    None => false,
                },
                None => false,
            };
            if !satisfied {
                result = result.with(dir);
            }
        }
        result
    }

    /// Whether every arrow of every piece is met by the opposite arrow of
    /// the piece it points at: a completed web of mutual pairs.
    pub fn is_solved(&self) -> bool {
        (0..self.pieces.len()).all(|piece| self.satisfied(piece))
    }
```

Update the module doc (lines 1-7): "…each piece carrying arrows that must all be met by the opposite arrow on the piece they point at." (keep the rest of the paragraph).

- [ ] **Step 4: Run the game tests**

Run: `cargo test --lib game`
Expected: PASS — the existing `line_game` fixtures stay valid because the winning placement is already mutual ({Up} against {Down} and vice versa).

- [ ] **Step 5: Run the full suite**

Run: `cargo test --lib && cargo clippy -- -D warnings && cargo fmt --check`
Expected: PASS, including the widget tests (their boards come from the generator and `play_solution` now plays strict solutions).

- [ ] **Step 6: Commit**

```bash
rtk git add src/game.rs
rtk git commit -m "feat(game): require reciprocal arrows for satisfied arrows and the win"
```

---

### Task 4: Docs — README, AGENTS.md, CHANGELOG, widget, Params

**Files:**
- Modify: `README.md` (rule sentence lines 18-20; generation section lines 96-106)
- Modify: `AGENTS.md` (rule sentence lines 14-17; solver bullet line 53-55; generator bullet lines 57-61)
- Modify: `CHANGELOG.md` (`## [Unreleased]` section, line 8)
- Modify: `src/widget.rs` (doc comments at lines 176-178, 220-222, 227-229)
- Modify: `src/game.rs` (`Params::max_arrows` doc, line 178-179)

**Interfaces:**
- Consumes: the implemented rule from Tasks 1-3.
- Produces: documentation consistent with the code; no behavior changes.

- [ ] **Step 1: README rule sentence**

Replace lines 18-20:

```markdown
**Drag the pieces to any free node, or onto another piece to swap the
two** until every arrow of every piece is met by the opposite arrow of the
piece it points at: a completed web of mutual pairs where nothing points
into the void.
```

Replace the generation section body (lines 96-98):

```markdown
The generator lays a solved configuration down first: it picks which nodes are
occupied, then gives each piece arrows that provably point at other pieces in
that configuration, and closes the sets under reciprocity so every arrow is
mutually answered. A solution therefore always exists, by construction.
```

and the repair sentence (line 104): "if it finds more than one, it adds an arrow *pair* — a new arrow plus the opposite arrow on the piece it points at — that provably kills one of them (adding arrows can never break the configuration it was built from, and can never create a new solution), and recounts, until exactly one remains."

- [ ] **Step 2: AGENTS.md**

Replace lines 14-17: "Every piece carries 1-6 arrows pointing along the six lattice directions. Pieces move by drag & drop, to any free node or onto an occupied node to swap the two; the puzzle is solved when **every arrow of every piece points at an occupied node whose piece points back with the opposite arrow** (mutual pairs)."

Solver bullet: append "; a seated piece must also form mutual arrow pairs with its already-seated neighbors" to the description of the assignment step. Generator bullet: change "repairing non-unique candidates by adding arrows" to "repairing non-unique candidates by adding arrow pairs (new arrow plus the opposite arrow on its target)".

- [ ] **Step 3: CHANGELOG**

Under `## [Unreleased]`, add:

```markdown
### Changed

- An arrow is now satisfied only when the piece it points at points back
  with the opposite arrow: a solved board is a web of mutual arrow pairs.
  The solver, the generator, and the live accent-color feedback all follow
  the stricter rule. `Params::max_arrows` is now a soft upper bound for the
  generator's initial draw, since closing the constructed solution under
  reciprocity can add further arrows.
```

- [ ] **Step 4: widget.rs doc comments**

Lines 176-178, replace "the ones still pointing at an empty node or off the board are drawn in `unsatisfied_color`" with "the ones still pointing at an empty node or off the board, or meeting no reciprocal arrow, are drawn in `unsatisfied_color`". Lines 220-222: "Color for arrows that point at an occupied node." → "Color for arrows that are met by a reciprocal arrow." Lines 227-229: "Color for arrows that point at an empty node or off the board." → "Color for arrows that point at an empty node, off the board, or at a piece that does not point back."

- [ ] **Step 5: Params::max_arrows doc**

In `src/game.rs`, change the `max_arrows` field doc (lines 178-179) to: "/// Inclusive upper bound of arrows per piece (at most 6). A soft cap: the generator's reciprocal closure may push a piece above it."

- [ ] **Step 6: Verify and commit**

Run: `cargo check && cargo test --lib && cargo clippy -- -D warnings && cargo fmt --check && cargo doc --no-deps`
Expected: all PASS, no doc warnings.

```bash
rtk git add README.md AGENTS.md CHANGELOG.md src/widget.rs src/game.rs
rtk git commit -m "docs: describe the mutual-arrows rule"
```

---

### Task 5: Full verification + difficulty survey

**Files:**
- None modified (measurement only; preset tuning is a follow-up decision)

- [ ] **Step 1: Full check suite**

Run: `cargo check && cargo test --lib && cargo clippy -- -D warnings && cargo fmt --check`
Expected: all PASS.

- [ ] **Step 2: Offline survey**

Run: `cargo test --lib --release -- --ignored --nocapture`
Expected: `unique == samples` for all three presets (the test asserts it). Report the average arrows/piece and total time — reciprocity closure adds arrows, so expect the average to rise; report numbers to the user before considering any preset tuning.

- [ ] **Step 3: Report**

Summarize survey numbers and confirm the presets still generate quickly; flag whether `min_arrows`/`max_arrows` presets need tuning as a follow-up (do not tune without user approval).
