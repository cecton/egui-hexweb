# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](keep_a_changelog) and this project adheres to [Semantic
Versioning](semver).

## [Unreleased]

## [0.2.0] - 2026-09-08

### Changed

- **Breaking:** `symmetric_board(nodes)` is now `random_symmetric_board(nodes, seed)`.
  Instead of one fixed shape per node count, the seed picks uniformly among every
  mirror-symmetric connected shape of that size (subsets of `hexagon(2)`, holes allowed):
  426 shapes at 8 nodes, 989 at 12, 183 at 16.

### Added

- Initial release: `HexwebGame` (game logic), `HexwebWidget` (egui widget), `Dir`, `Arrows`,
  `Piece`, `GameStatus`, `content_size` and `fit_cell_size`

[keep_a_changelog]: https://keepachangelog.com/en/1.1.0/
[semver]: https://semver.org/spec/v2.0.0.html
