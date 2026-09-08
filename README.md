# egui-hexweb

[![crates.io](https://img.shields.io/crates/v/egui-hexweb.svg)](https://crates.io/crates/egui-hexweb)
[![docs.rs](https://docs.rs/egui-hexweb/badge.svg)](https://docs.rs/egui-hexweb)
[![deps.rs](https://deps.rs/repo/github/cecton/egui-hexweb/status.svg)](https://deps.rs/repo/github/cecton/egui-hexweb)
[![CI](https://github.com/cecton/egui-hexweb/actions/workflows/ci.yml/badge.svg)](https://github.com/cecton/egui-hexweb/actions/workflows/ci.yml)
[![Rust version](https://img.shields.io/badge/rustc-1.80+-ab6000.svg)](https://blog.rust-lang.org/2024/07/25/Rust-1.80.0.html)
[![License](https://img.shields.io/crates/l/egui-hexweb.svg)](https://github.com/cecton/egui-hexweb#license)
[![Changelog](https://img.shields.io/badge/changelog-Keep%20a%20Changelog%20v1.1.0-%23E05735)](CHANGELOG.md)
[![Live demo](https://img.shields.io/badge/demo-live-brightgreen)](https://cecton.github.io/egui-hexweb)

A self-contained hexagonal arrow-connection puzzle game library for
[egui](https://github.com/emilk/egui).

The board is a small patch of hexagonal lattice: nodes joined by grid lines in
six directions (up, down, and the four diagonals). Fewer pieces than nodes sit
on the board, and each piece carries a few arrows pointing along those grid
lines. **Drag the pieces to any free node** until every arrow of every piece
points at a node that holds another piece: a completed web where nothing
points into the void.

Unsatisfied arrows are drawn in an accent color, so the board itself always
tells you how close you are.

## Features

- Pure game logic struct (`HexwebGame`) with no `egui::Ui` dependency, usable headlessly or with any renderer
- Ready-to-use egui `Widget` (`HexwebWidget`) with drag & drop plus a click-to-select fallback
- Procedural, seeded generation (`HexwebGame::random`) that always produces a solvable board and verifies, per board, that **exactly one** placement of the pieces solves it
- An exact counting solver (`HexwebGame::solution_count`, `HexwebGame::solution`), exhaustive over the whole placement space, fast enough to run on every generated candidate and after every repair
- Mirror-symmetric board shapes of 8 to 16 nodes (`symmetric_board`), plus plain `hexagon` boards
- No losing state and no timer: every move is undone by moving back
- The initial scramble is guaranteed to be neither solved nor solvable in a single move

## Usage

Add the dependency:

```toml
[dependencies]
egui-hexweb = "0.1"
```

Then use it in your egui app:

```rust,ignore
use egui_hexweb::{symmetric_board, HexwebGame, HexwebWidget, Params};

// A mirror-symmetric 12-node board with 9 pieces, 2-4 arrows each,
// reproducible from a seed.
let mut game = HexwebGame::random(
    Params {
        nodes: symmetric_board(12),
        pieces: 9,
        min_arrows: 2,
        max_arrows: 4,
    },
    42,
);

// Inside your egui update/UI closure:
ui.add(HexwebWidget::new(&mut game));

// Customize the win banner:
ui.add(HexwebWidget::new(&mut game).win_message("Solved!"));
```

Check for a win after each frame:

```rust,ignore
use egui_hexweb::GameStatus;

match game.status() {
    GameStatus::InProgress => {}
    GameStatus::Won => println!("Every arrow is connected!"),
}
```

To start over on the same puzzle:

```rust,ignore
game.reset();
```

To offer a hint, ask the solver where the pieces belong:

```rust,ignore
if let Some(nodes) = game.solution() {
    // `nodes[piece]` is the node piece `piece` occupies in one solution.
}
```

## How generation guarantees a single solution

The generator lays a solved configuration down first: it picks which nodes are
occupied, then gives each piece arrows that provably point at other pieces in
that configuration. A solution therefore always exists, by construction.

Because the boards are small (8 to 16 nodes), the solver is exhaustive rather
than heuristic: it enumerates every possible set of empty nodes and counts the
distinct assignments of the pieces that satisfy every arrow, treating
identical pieces as interchangeable. The generator counts up to two solutions;
if it finds more than one, it adds an arrow that provably kills one of them
(adding an arrow can never break the configuration it was built from, and can
never create a new solution), and recounts, until exactly one remains.

## egui version compatibility

| egui-hexweb | egui |
|-------------|------|
| 0.1         | 0.35 |

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.
