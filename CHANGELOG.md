# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](keep_a_changelog) and this project adheres to [Semantic
Versioning](semver).

## [Unreleased]

## [0.5.0] - 2026-09-19

### Removed

- The web demo's drag-to-pan (and pinch/ctrl-wheel zoom) view on narrow viewports and touch
  devices, inherited from the egui-minesweeper template where big boards need it. Hexweb boards
  always fit the viewport, so the board is now laid out directly and centered instead of living in
  an `egui::Scene`

### Changed

- Updated egui to 0.36.
- The test harness now clears egui's `textures_delta` after each pass, as egui
  0.36 panics when unapplied texture deltas are dropped.

## [0.4.0] - 2026-09-08

### Removed

- The click-to-select fallback: drag & drop is now the only input, and a
  plain click does nothing. Touch devices drag natively, and the game has no
  losing state, so there is nothing a second input style could protect
  against. The `HexwebWidget` now senses only drags; no public items changed

## [0.3.0] - 2026-09-08

### Added

- `HexwebGame::can_swap` and `HexwebGame::swap_pieces`: two placed pieces
  exchange nodes, counting a move like any other. Two pieces with identical
  arrows are interchangeable, so swapping those is a free no-op that costs
  no move
- Landing rings while a piece is dragged now also mark the pieces a drop
  would swap with, drawn on top of them
- The scramble guarantee extends to swaps: a generated board is never one
  move *or one swap* away from solved

### Changed

- Dropping a dragged piece onto an occupied node swaps the two pieces
  instead of snapping back

## [0.2.0] - 2026-09-08

### Added

- Initial release: `HexwebGame` (game logic), `HexwebWidget` (egui widget), `Dir`, `Arrows`,
  `Piece`, `GameStatus`, `content_size` and `fit_cell_size`
- `HexwebGame::random` generates a seeded puzzle that always has a solution by
  construction and verifies, with an exact counting solver, that exactly one placement of
  the pieces solves it. The scramble is neither solved nor one move away from solved
- `random_symmetric_board(nodes, seed)`: a random mirror-symmetric board per seed, picked
  uniformly among every connected shape of that size over `hexagon(2)` (holes allowed):
  426 shapes at 8 nodes, 989 at 12, 183 at 16
- Live feedback while playing: unsatisfied arrows are drawn in the accent color, and the
  win is latched behind an in-widget banner

[keep_a_changelog]: https://keepachangelog.com/en/1.1.0/
[semver]: https://semver.org/spec/v2.0.0.html
[Unreleased]: https://github.com/cecton/egui-hexweb/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/cecton/egui-hexweb/releases/tag/v0.4.0
[0.3.0]: https://github.com/cecton/egui-hexweb/releases/tag/v0.3.0
[0.2.0]: https://github.com/cecton/egui-hexweb/releases/tag/v0.2.0
