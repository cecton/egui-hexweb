# AGENTS.md

Instructions for AI coding agents working in this repository.

## What this is

`egui-hexweb` is a self-contained Rust library implementing a hexagonal
arrow-connection puzzle for [egui](https://github.com/emilk/egui):
renderer-agnostic game logic plus a ready-to-use `egui::Widget`. It has no
application of its own beyond the demo in `examples/webapp.rs` — it's meant to
be pulled into other egui apps as a dependency.

The puzzle: a small patch of hexagonal lattice with fewer pieces than nodes.
Every piece carries 1-6 arrows pointing along the six lattice directions.
Pieces move by drag & drop, to any free node or onto an occupied node to
swap the two; the puzzle is solved when
**every arrow of every piece points at an occupied node**.

"Hexa Arrows" is a trademarked product name of an existing mobile puzzle —
never use it, or names confusingly close to it, in code, docs, or naming.
This project only refers to the genre generically ("arrow-connection
puzzle", "hexweb"). The same caution applies to "Hexcells", which names a
*different* hexagonal puzzle genre.

## The model everything rests on

A board is just a set of axial coordinates on the flat-top hex lattice.
Directions are `Up (0,-1)`, `Down (0,+1)`, `RightUp (+1,-1)`, `RightDown
(+1,0)`, `LeftUp (-1,0)`, `LeftDown (-1,+1)`; pixel positions come from
`x = 1.5*size*q`, `y = sqrt(3)*size*(r + q/2)`.

A piece *is* its arrow set (`Arrows`, a bitmask): two pieces with the same
arrows are interchangeable, and the uniqueness guarantee is stated in terms of
the **multiset** of arrow sets — permuting identical pieces does not create a
second solution. This is load-bearing in the solver (groups pick nodes in
increasing order, collapsing permutations) and in every test that asserts a
count.

Boards are small by design (8-16 nodes). That is what makes the solver
exhaustive: enumerating the empty-node complements peaks at C(16,4) = 1820
occupancy sets, so exact counting is microseconds and the generator can
afford to verify after every edit. Don't "scale it up" without revisiting
that; the u64 occupancy mask caps boards at 64 nodes.

## Module layout

- `src/game.rs` — `Dir`, `Arrows`, `Piece`, `GameStatus`, `Node`, `Params`,
  `HexwebGame`, the shape constructors (`hexagon`, `random_symmetric_board`). Pure
  logic, no `egui::Widget`/`Ui` usage. Keep it that way: it should stay usable
  headlessly (for tests, or a non-egui renderer) without pulling in painting
  code.
- `src/solver.rs` — `pub(crate)` counting over the placement space: enumerate
  empty-node complements, then assign arrow-set groups to fitting nodes in
  increasing node order. It counts **placements of the multiset**, not
  permutations of identical pieces. Don't "optimize" that away; it is the
  property the uniqueness guarantee is stated in terms of.
- `src/generator.rs` — `pub(crate)` only. Lays a solved configuration first,
  verifies with the counting solver, repairs non-unique candidates by adding
  arrows (monotone: can never break the built solution or create new
  solutions), then scrambles by a random walk of legal moves. The module doc
  carries the full argument for why repair terminates and why the scramble
  walk keeps the solution reachable.
- `src/widget.rs` — `HexwebWidget`, `content_size`, `fit_cell_size`, and all
  painting/input handling. The only file allowed to depend on
  `egui::Ui`/`Painter`.
- `src/lib.rs` — thin re-export surface. `#![doc = include_str!("../README.md")]`
  means the crate-level docs are the README; keep the two in sync (usage
  snippets especially).
- `examples/webapp.rs` — a wasm demo app (via `xtask-wasm`), deployed to
  GitHub Pages by `.github/workflows/deploy.yml` on every push to `main`. Not
  part of the published crate (`Cargo.toml` excludes `/examples`).

## Building and testing

```sh
cargo check
cargo test --lib
cargo clippy -- -D warnings
cargo fmt --check
```

These four are exactly what `.github/workflows/ci.yml` runs on every push and
PR. Run them locally before committing.

The wasm demo isn't covered by `ci.yml` (only `deploy.yml` builds it, on push
to `main`). If you touch `examples/webapp.rs`, check it manually:

```sh
cargo check --target wasm32-unknown-unknown --example webapp
cargo clippy --target wasm32-unknown-unknown --example webapp -- -D warnings
cargo run --example webapp -- start     # local dev server, to actually play it
```

There is also an opt-in offline survey in `generator.rs` (every test
`#[ignore]`d) that measures the uniqueness hit-rate and repair counts at a
sample size the normal suite can't afford:

```sh
cargo test --lib --release -- --ignored --nocapture
```

## Conventions

- **No losing state and no timer.** Every move is reversible by moving back.
  If a scoring or challenge mode is ever added, it must be additive.
- A win is **latched**: once solved the board stops accepting moves, so the
  banner doesn't flicker away. Tests that move pieces must not assume
  `move_piece` keeps succeeding.
- `HexwebGame::random` must always produce a board that has *at least* one
  solution by construction (the generator's own layout is one), and prefers
  one verified to have exactly one within `ATTEMPTS`. The scramble must be
  neither solved nor one move or swap away from solved. All three properties
  have tests; keep them.
- Generation is seeded and reproducible. Don't introduce unseeded randomness
  or time-dependence into `game.rs`/`generator.rs`.
- Add unit tests in `src/game.rs` for player-facing behavior (moves, win
  detection, latch, reset, shape symmetry), in `src/solver.rs` for counting
  (hand-built boards with 0, 1 and 2 known solutions, brute-force
  cross-checks), and in `src/generator.rs` for generation properties (unique,
  not already solved, no one-move win). The painting code isn't unit testable
  the same way beyond its geometry helpers; verify it by eye via the wasm
  demo.
- Keep the public API renderer-agnostic where possible: prefer exposing
  queries (`piece_at`, `unsatisfied_arrows`, `solution`) over raw field
  access, so the internal representation can change without breaking callers.
- The widget displays text hosts pass in (`win_message`); the crate ships no
  translations. Keep new user-facing knobs as builder methods taking
  `impl Into<String>`.

## Release process

Every published version gets a git tag and a changelog entry. To cut a
release:

1. Update `CHANGELOG.md`: move the `[Unreleased]` section's contents under a
   new `## [X.Y.Z] - YYYY-MM-DD` heading (Keep a Changelog format), and add
   the corresponding link reference at the bottom of the file.
2. Bump the `version` in `Cargo.toml` to match.
3. Run the full check suite above, plus `cargo package --list` as a final
   sanity check of what will actually be published.
4. `cargo publish`. This is irreversible per-version (a bad release can only
   be `cargo yank`-ed, not deleted) — don't skip step 3.
5. `git tag vX.Y.Z && git push && git push --tags`.

Follow SemVer: breaking changes (renamed/removed public items, changed method
signatures) require a major version bump (or a minor bump pre-1.0, per
SemVer's pre-1.0 rules).
