# Mutual arrows rule — design

Date: 2026-10-02
Status: approved

## Problem

The game currently counts an arrow as satisfied when it points at an occupied
node, regardless of whether the piece on that node points back. The puzzle
should be a web of **mutual arrow pairs**: an arrow is satisfied only if its
target node is occupied *and* the piece there carries the opposite arrow.

## Decisions (from brainstorming)

1. **Rule**: solved iff every arrow of every piece is half of a mutual pair.
   Feedback (`unsatisfied_arrows`), solver, and generator all follow it.
2. **`max_arrows` becomes a soft cap**: the generator's reciprocal closure may
   push a piece above `Params::max_arrows`; documented, no retry loops.
3. **Solver**: keep occupancy-first enumeration and group seating; add a
   pairwise mutual check while seating (not a leaf-only check, not a rewrite).

## Rule

For a placement, piece `p` on node `n` with arrow `d` is satisfied iff:

- `nodes[n].neighbors[d]` is in-board **and** occupied, **and**
- the piece `q` on that neighbor satisfies `q.arrows.contains(d.opposite())`.

The board is solved iff every arrow of every piece is satisfied. Everything
else (reversible moves, latched win, no timer) is unchanged.

## game.rs

`unsatisfied_arrows` gains one step after the existing occupied check: if the
target is occupied, mark the arrow unsatisfied when the occupant's arrow set
lacks `dir.opposite()`. `satisfied`, `is_solved`, the widget's accent-color
feedback, and the win latch all derive from it unchanged.

Existing test fixtures keep working: in `line_game`, the winning move puts
piece0 {Up} against piece1 {Down} and vice versa — already mutual. New tests:
occupied-but-not-reciprocal arrow reports unsatisfied; mutual pair reports
satisfied; a board with one non-mutual pair is not solved.

## solver.rs

The old "fit is purely local" property breaks (satisfaction now depends on
which piece sits at the target), so reciprocity is enforced during seating:

- New `group_at` buffer (node id → seated group index), maintained alongside
  `slots` (push/pop in the DFS).
- When seating node `n` for group `gi`: for each of the ≤6 neighbor directions
  `d`, if that neighbor is already seated with group `g2`, require
  `gi.contains(d) == g2.contains(d.opposite())`. Neither side pointing is
  fine; exactly one side pointing fails.
- Every adjacent seated pair is checked exactly once — when its second
  endpoint is seated — so any completed seating is fully mutual, and bad
  branches prune before the leaf.

`count`, `solution`, `alternate_solution` keep their signatures. The multiset
grouping (groups pick nodes in increasing order) is untouched.

Test updates: the naive reference `naive_count` implements the same rule;
`[Up, Up, Down]` on the 3-line drops from 1 solution to 0, and a
`[Down, Up, Up]` fixture keeps the grouping/canonicalization coverage at 1;
`solution_is_actually_a_solution` additionally asserts reciprocity; the
brute-force cross-check compares the two updated implementations.

## generator.rs

- `assign_arrows`: after the random draw, one closure pass — for each arrow
  `d` of piece `i`, the piece on the target node gains `d.opposite()` if
  missing. One pass suffices: every added arrow points back at a piece that
  already points here, so it never creates a new missing reciprocal.
- `repair_until_unique`: a repair adds killer arrow `d` to piece `p` *and*
  `d.opposite()` to the target piece (when absent). Monotonicity survives:
  the constructed configuration stays valid (both new arrows are mutual
  within it) and the constraint strictly strengthens, so the solution count
  never rises and never reaches zero. Kill detection considers both added
  arrows when preferring killers.
- `is_solved_placement` (scramble / one-move / one-swap checks) applies the
  same reciprocity condition as the solver's seating check.
- Module doc: step 2 gains the closure, step 3 the paired repair, and the
  monotone argument is restated for pairs.

## Docs

README (crate-level docs), AGENTS.md rule sentence, `widget.rs` doc comment,
`Params` field docs (soft-cap note), CHANGELOG `[Unreleased]`.

## Verification

- `cargo check`, `cargo test --lib`, `cargo clippy -- -D warnings`,
  `cargo fmt --check`.
- Rerun the ignored survey (`cargo test --lib --release -- --ignored
  --nocapture`): closure adds arrows, so average arrows/piece rises; presets
  may warrant tuning afterward — separate decision, reported with numbers.
- Widget tests: verified during planning that none of the hand-built widget
  fixtures call `solution()` (only generated preset boards do, via
  `play_solution`), so no fixture change is needed; only doc comments in
  `widget.rs` mention the rule and get updated.

## Non-goals

- No new game modes, scoring, or timers.
- No solver enumeration rewrite; board sizes stay 8–16 nodes.
- No hard enforcement of `max_arrows` under closure.
