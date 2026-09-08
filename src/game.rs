//! Game logic for the hexweb puzzle: a hexagonal lattice of nodes where a
//! subset of nodes holds pieces, each piece carrying arrows that must all
//! point at occupied nodes.
//!
//! This module is pure logic: no `egui` types, no painting, no input. The
//! [`HexwebGame`] struct owns the board and the piece placement; the widget
//! in `crate::widget` renders and drives it.

use std::collections::BTreeMap;

/// Index into the board's node list.
pub type NodeId = usize;

/// Index into the board's piece list.
pub type PieceId = usize;

/// One of the six lattice directions of the flat-top hex grid, ordered
/// clockwise starting at the top.
///
/// In axial coordinates the deltas are:
///
/// | direction   | delta     | pixel delta (size = circumradius)     |
/// |-------------|-----------|---------------------------------------|
/// | [`Dir::Up`]      | `(0, -1)` | `(0, -√3·size)`                  |
/// | [`Dir::RightUp`] | `(1, -1)` | `(1.5·size, -√3/2·size)`         |
/// | [`Dir::RightDown`]| `(1, 0)` | `(1.5·size, +√3/2·size)`         |
/// | [`Dir::Down`]    | `(0, +1)` | `(0, +√3·size)`                  |
/// | [`Dir::LeftDown`]| `(-1, +1)`| `(-1.5·size, +√3/2·size)`        |
/// | [`Dir::LeftUp`]  | `(-1, 0)` | `(-1.5·size, -√3/2·size)`        |
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
pub enum Dir {
    Up = 0,
    RightUp,
    RightDown,
    Down,
    LeftDown,
    LeftUp,
}

impl Dir {
    /// All directions, clockwise. The index in this array is the bit index
    /// used by [`Arrows`] and the angle used when painting an arrow.
    pub const ALL: [Dir; 6] = [
        Dir::Up,
        Dir::RightUp,
        Dir::RightDown,
        Dir::Down,
        Dir::LeftDown,
        Dir::LeftUp,
    ];

    /// Axial-coordinate delta of this direction.
    pub fn delta(self) -> (i32, i32) {
        match self {
            Dir::Up => (0, -1),
            Dir::RightUp => (1, -1),
            Dir::RightDown => (1, 0),
            Dir::Down => (0, 1),
            Dir::LeftDown => (-1, 1),
            Dir::LeftUp => (-1, 0),
        }
    }

    /// The direction pointing the opposite way.
    pub fn opposite(self) -> Dir {
        match self {
            Dir::Up => Dir::Down,
            Dir::RightUp => Dir::LeftDown,
            Dir::RightDown => Dir::LeftUp,
            Dir::Down => Dir::Up,
            Dir::LeftDown => Dir::RightUp,
            Dir::LeftUp => Dir::RightDown,
        }
    }
}

/// A non-empty subset of the six directions, as a bitmask.
///
/// Two pieces with equal `Arrows` are interchangeable: the uniqueness
/// guarantee counts placements of the *multiset* of arrow sets, so swapping
/// identical pieces is not a second solution.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash, Default)]
pub struct Arrows(u8);

impl Arrows {
    /// The set containing exactly one direction.
    pub fn from_dir(dir: Dir) -> Self {
        Self(1 << dir as u8)
    }

    /// The set containing all six directions.
    pub fn all() -> Self {
        Self(0b0011_1111)
    }

    /// Whether `dir` is in the set.
    pub fn contains(self, dir: Dir) -> bool {
        self.0 & (1 << dir as u8) != 0
    }

    /// The set plus `dir`.
    pub fn with(self, dir: Dir) -> Self {
        Self(self.0 | (1 << dir as u8))
    }

    /// The set minus `dir`.
    pub fn without(self, dir: Dir) -> Self {
        Self(self.0 & !(1 << dir as u8))
    }

    /// How many directions the set holds.
    pub fn len(self) -> usize {
        self.0.count_ones() as usize
    }

    /// Whether the set is empty.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The raw bitmask. Grouping/sorting by this value gives every distinct
    /// arrow set a fixed canonical order.
    pub(crate) fn bits(self) -> u8 {
        self.0
    }

    /// The directions, clockwise.
    pub fn iter(self) -> impl Iterator<Item = Dir> {
        Dir::ALL.into_iter().filter(move |&dir| self.contains(dir))
    }
}

impl IntoIterator for Arrows {
    type Item = Dir;
    type IntoIter = std::vec::IntoIter<Dir>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter().collect::<Vec<_>>().into_iter()
    }
}

/// A movable piece. A piece is fully described by its arrows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Piece {
    pub arrows: Arrows,
}

/// Whether the puzzle is solved. The win is latched: it never reverts.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GameStatus {
    #[default]
    InProgress,
    Won,
}

/// A node of the board with its axial coordinates and precomputed neighbors.
#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub q: i32,
    pub r: i32,
    /// Neighbor in each direction, if that node exists on the board.
    pub neighbors: [Option<NodeId>; 6],
}

/// Parameters for [`HexwebGame::random`].
#[derive(Clone, Debug)]
pub struct Params {
    /// Axial coordinates of the board's nodes, e.g. from [`symmetric_board`].
    pub nodes: Vec<(i32, i32)>,
    /// How many pieces the board holds; the rest of the nodes stay empty.
    /// Must be at least 1 and strictly less than `nodes.len()` (there must be
    /// somewhere to move to).
    pub pieces: usize,
    /// Inclusive lower bound of arrows per piece (at least 1).
    pub min_arrows: usize,
    /// Inclusive upper bound of arrows per piece (at most 6).
    pub max_arrows: usize,
}

/// The hexweb puzzle: pieces on a hexagonal lattice, moved until every arrow
/// points at another piece.
#[derive(Clone, Debug)]
pub struct HexwebGame {
    nodes: Vec<Node>,
    /// Coordinate -> node id. A BTreeMap so `Debug` output (and thus the
    /// reproducibility tests) is deterministic.
    lookup: BTreeMap<(i32, i32), NodeId>,
    pieces: Vec<Piece>,
    /// Occupancy: which piece (if any) sits on each node.
    cell_piece: Vec<Option<PieceId>>,
    /// The placement the board starts (and resets) with.
    initial: Vec<Option<PieceId>>,
    moves: usize,
    status: GameStatus,
}

/// All nodes `(q, r)` with hex distance at most `radius` from the center:
/// `3·R·(R+1)+1` nodes (1, 7, 19, 37, ...).
pub fn hexagon(radius: u32) -> Vec<(i32, i32)> {
    let r = radius as i32;
    let mut coords = Vec::new();
    for q in -r..=r {
        for rr in (-r).max(-q - r)..=r.min(-q + r) {
            coords.push((q, rr));
        }
    }
    coords
}

/// A mirror-symmetric board with exactly `nodes` nodes, for any count from 8
/// to 16.
///
/// Plain hexagons only exist at 7, 19, 37, ... nodes, so the shapes in between
/// grow the 7-node hexagon with whole mirror pairs (and on-axis nodes) with
/// respect to the vertical axis `(q, r) -> (-q, r+q)`. Every shape is
/// connected and mirror-symmetric; 13 nodes is even six-fold symmetric (the
/// "flower": the 7-node hexagon plus all six ring-2 edge midpoints).
///
/// Panics for node counts outside `8..=16`.
pub fn symmetric_board(nodes: usize) -> Vec<(i32, i32)> {
    match nodes {
        8..=15 => {
            let mut extra: Vec<(i32, i32)> = match nodes {
                8 => vec![(0, -2)],
                9 => pm1(),
                10 => [vec![(0, -2)], pm1()].concat(),
                11 => [pm1(), pc1()].concat(),
                12 => vec![(0, -2), (1, -2), (2, -1), (-2, 1), (-1, -1)],
                13 => [pm1(), pm2(), pm3()].concat(),
                14 => [[pm1(), pm2(), pm3()].concat(), vec![(0, -2)]].concat(),
                15 => [[pm1(), pm2(), pm3()].concat(), vec![(0, -2), (0, 2)]].concat(),
                _ => unreachable!(),
            };
            extra.extend(hexagon(1));
            extra
        }
        16 => {
            // hexagon(2) minus a symmetric bite out of the bottom edge.
            hexagon(2)
                .into_iter()
                .filter(|&c| !matches!(c, (0, 2) | (2, 0) | (-2, 2)))
                .collect()
        }
        _ => panic!("symmetric_board supports 8..=16 nodes, got {nodes}"),
    }
}

fn pm1() -> Vec<(i32, i32)> {
    vec![(1, -2), (-1, -1)]
}

fn pm2() -> Vec<(i32, i32)> {
    vec![(2, -1), (-2, 1)]
}

fn pm3() -> Vec<(i32, i32)> {
    vec![(1, 1), (-1, 2)]
}

fn pc1() -> Vec<(i32, i32)> {
    vec![(2, -2), (-2, 0)]
}

/// Builds the node list with precomputed neighbor tables from raw
/// coordinates.
pub(crate) fn build_nodes(coords: &[(i32, i32)]) -> (Vec<Node>, BTreeMap<(i32, i32), NodeId>) {
    let lookup: BTreeMap<(i32, i32), NodeId> = coords
        .iter()
        .enumerate()
        .map(|(id, &coord)| (coord, id))
        .collect();
    let nodes = coords
        .iter()
        .map(|&(q, r)| Node {
            q,
            r,
            neighbors: std::array::from_fn(|i| {
                let (dq, dr) = Dir::ALL[i].delta();
                lookup.get(&(q + dq, r + dr)).copied()
            }),
        })
        .collect();
    (nodes, lookup)
}

impl HexwebGame {
    /// Generates a new puzzle from `params` with a deterministic seed.
    ///
    /// The generator lays a solved configuration down first, then verifies
    /// with an exact counting solver that exactly one placement of the pieces
    /// solves it (repairing by adding arrows if not), and finally scrambles
    /// the board with legal moves. See `crate::generator`.
    ///
    /// Panics (see [`Params`] and [`validate_params`]) on impossible
    /// parameters: duplicate or isolated nodes, a piece count outside
    /// `1..nodes.len()`, or an arrow range outside `1..=6` with
    /// `min <= max`.
    pub fn random(params: Params, seed: u64) -> Self {
        validate_params(&params);
        let (nodes, _) = build_nodes(&params.nodes);
        let mut rng = fastrand::Rng::with_seed(seed);
        let generated = crate::generator::generate(
            &nodes,
            params.pieces,
            params.min_arrows,
            params.max_arrows,
            &mut rng,
        );
        let pieces: Vec<Piece> = generated
            .arrows
            .into_iter()
            .map(|arrows| Piece { arrows })
            .collect();
        Self::from_parts(
            nodes,
            pieces,
            generated.scramble.clone(),
            generated.scramble,
        )
    }

    pub(crate) fn from_parts(
        nodes: Vec<Node>,
        pieces: Vec<Piece>,
        cell_piece: Vec<Option<PieceId>>,
        initial: Vec<Option<PieceId>>,
    ) -> Self {
        debug_assert_eq!(nodes.len(), cell_piece.len());
        debug_assert_eq!(cell_piece.len(), initial.len());
        let lookup: BTreeMap<(i32, i32), NodeId> = nodes
            .iter()
            .enumerate()
            .map(|(id, node)| ((node.q, node.r), id))
            .collect();
        Self {
            nodes,
            lookup,
            pieces,
            cell_piece,
            initial,
            moves: 0,
            status: GameStatus::InProgress,
        }
    }

    /// The board's nodes, in the order of the coordinates passed to
    /// [`Params`].
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// Total node count (occupied + empty).
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// How many pieces the board holds.
    pub fn piece_count(&self) -> usize {
        self.pieces.len()
    }

    /// How many nodes are currently empty.
    pub fn empty_count(&self) -> usize {
        self.nodes.len() - self.pieces.len()
    }

    /// The pieces, in piece-id order.
    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    /// The piece sitting on `node`, if any.
    pub fn piece_at(&self, node: NodeId) -> Option<PieceId> {
        self.cell_piece[node]
    }

    /// The node `piece` currently occupies.
    pub fn node_of(&self, piece: PieceId) -> Option<NodeId> {
        self.cell_piece
            .iter()
            .position(|&occupant| occupant == Some(piece))
    }

    /// The node with axial coordinates `(q, r)`, if it exists on the board.
    pub fn node_at(&self, q: i32, r: i32) -> Option<NodeId> {
        self.lookup.get(&(q, r)).copied()
    }

    /// Whether `piece` may be moved to `to` right now: the game must be
    /// unfinished, `to` must be an in-board node, and it must be empty and
    /// not `piece`'s own node.
    pub fn can_move(&self, piece: PieceId, to: NodeId) -> bool {
        if self.status != GameStatus::InProgress || to >= self.nodes.len() {
            return false;
        }
        match self.node_of(piece) {
            Some(from) => from != to && self.cell_piece[to].is_none(),
            None => false,
        }
    }

    /// Moves `piece` to the empty node `to`. Returns whether the move
    /// happened; illegal moves (see [`HexwebGame::can_move`]) leave the board
    /// unchanged. Counts the move and latches the win when the board becomes
    /// solved.
    pub fn move_piece(&mut self, piece: PieceId, to: NodeId) -> bool {
        let Some(from) = self.node_of(piece) else {
            return false;
        };
        if !self.can_move(piece, to) {
            return false;
        }
        self.cell_piece[from] = None;
        self.cell_piece[to] = Some(piece);
        self.moves += 1;
        if self.is_solved() {
            self.status = GameStatus::Won;
        }
        true
    }

    /// Whether every arrow of `piece` currently points at an occupied node.
    pub fn satisfied(&self, piece: PieceId) -> bool {
        self.unsatisfied_arrows(piece).is_empty()
    }

    /// The arrows of `piece` whose target node is empty or off the board.
    /// This is the live feedback the widget paints in the accent color.
    pub fn unsatisfied_arrows(&self, piece: PieceId) -> Arrows {
        let Some(node) = self.node_of(piece) else {
            return self.pieces[piece].arrows;
        };
        let mut result = Arrows::default();
        for dir in self.pieces[piece].arrows.iter() {
            match self.nodes[node].neighbors[dir as usize] {
                Some(neighbor) if self.cell_piece[neighbor].is_some() => {}
                _ => result = result.with(dir),
            }
        }
        result
    }

    /// Whether every arrow of every piece points at an occupied node.
    pub fn is_solved(&self) -> bool {
        (0..self.pieces.len()).all(|piece| self.satisfied(piece))
    }

    /// How many moves have been played since the last reset/new game.
    pub fn moves(&self) -> usize {
        self.moves
    }

    /// Whether the game is finished. Latched: once [`GameStatus::Won`], it
    /// stays won until [`HexwebGame::reset`] or a new game.
    pub fn status(&self) -> GameStatus {
        self.status
    }

    /// Restores the initial scramble, zeroing the move counter.
    pub fn reset(&mut self) {
        self.cell_piece = self.initial.clone();
        self.moves = 0;
        self.status = GameStatus::InProgress;
    }

    /// How many distinct placements of the pieces solve the board, capped at
    /// `cap`. Counts the multiset: identical pieces are interchangeable, so
    /// swapping two of them is not a second solution. Runs in microseconds
    /// on the shipped board sizes; see `crate::solver`.
    pub fn solution_count(&self, cap: usize) -> usize {
        crate::solver::count(&self.pieces, &self.nodes, cap)
    }

    /// One solving placement, as the node each piece occupies (indexed by
    /// piece id), or `None` if the board has no solution.
    pub fn solution(&self) -> Option<Vec<NodeId>> {
        crate::solver::solution(&self.pieces, &self.nodes)
    }
}

fn validate_params(params: &Params) {
    assert!(!params.nodes.is_empty(), "board needs at least one node");
    let mut sorted = params.nodes.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        params.nodes.len(),
        "board nodes must be distinct coordinates"
    );
    let (built, _) = build_nodes(&params.nodes);
    assert!(
        built
            .iter()
            .all(|node| node.neighbors.iter().any(Option::is_some)),
        "board has isolated nodes with no in-board neighbor"
    );
    assert!(
        params.pieces >= 1,
        "board needs at least one piece, got {}",
        params.pieces
    );
    assert!(
        params.pieces < params.nodes.len(),
        "board needs at least one empty node: {} pieces on {} nodes",
        params.pieces,
        params.nodes.len()
    );
    assert!(
        (1..=6).contains(&params.min_arrows)
            && (params.min_arrows..=6).contains(&params.max_arrows),
        "arrow range must be within 1..=6 with min <= max, got {}..={}",
        params.min_arrows,
        params.max_arrows
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3-node vertical line: node 0 at (0,0), node 1 at (0,-1) above it,
    /// node 2 at (0,1) below it. Piece 0 (arrow Up) starts on node 1, piece 1
    /// (arrow Down) on node 0, leaving node 2 empty. Both arrows are
    /// unsatisfied; moving piece 0 to node 2 solves it.
    fn line_game() -> HexwebGame {
        let (nodes, _) = build_nodes(&[(0, 0), (0, -1), (0, 1)]);
        let pieces = vec![
            Piece {
                arrows: Arrows::from_dir(Dir::Up),
            },
            Piece {
                arrows: Arrows::from_dir(Dir::Down),
            },
        ];
        let cell_piece = vec![Some(1), Some(0), None];
        HexwebGame::from_parts(nodes, pieces, cell_piece.clone(), cell_piece)
    }

    #[test]
    fn dir_delta_and_opposite_tables() {
        let expected = [
            (Dir::Up, (0, -1), Dir::Down),
            (Dir::RightUp, (1, -1), Dir::LeftDown),
            (Dir::RightDown, (1, 0), Dir::LeftUp),
            (Dir::Down, (0, 1), Dir::Up),
            (Dir::LeftDown, (-1, 1), Dir::RightUp),
            (Dir::LeftUp, (-1, 0), Dir::RightDown),
        ];
        for &(dir, delta, opposite) in &expected {
            assert_eq!(dir.delta(), delta);
            assert_eq!(dir.opposite(), opposite);
            assert_eq!(opposite.opposite(), dir);
        }
        assert_eq!(Dir::ALL.len(), 6);
    }

    #[test]
    fn arrows_bitmask_round_trip() {
        let mut arrows = Arrows::default();
        assert!(arrows.is_empty());
        assert_eq!(arrows.len(), 0);
        arrows = arrows.with(Dir::Up).with(Dir::LeftDown);
        assert_eq!(arrows.len(), 2);
        assert!(arrows.contains(Dir::Up));
        assert!(arrows.contains(Dir::LeftDown));
        assert!(!arrows.contains(Dir::Down));
        assert_eq!(
            arrows.iter().collect::<Vec<_>>(),
            vec![Dir::Up, Dir::LeftDown]
        );
        arrows = arrows.without(Dir::Up);
        assert_eq!(arrows, Arrows::from_dir(Dir::LeftDown));
        assert_eq!(Arrows::all().len(), 6);
    }

    #[test]
    fn hexagon_sizes() {
        assert_eq!(hexagon(0).len(), 1);
        assert_eq!(hexagon(1).len(), 7);
        assert_eq!(hexagon(2).len(), 19);
        assert!(hexagon(1).contains(&(0, 0)));
        assert!(hexagon(1).contains(&(-1, 1)));
        assert!(!hexagon(1).contains(&(0, 2)));
    }

    #[test]
    fn symmetric_board_shapes_are_symmetric_connected_and_sized() {
        for nodes in 8..=16 {
            let coords = symmetric_board(nodes);
            assert_eq!(coords.len(), nodes, "shape of {nodes} nodes");

            // Mirror invariance: (q, r) -> (-q, r+q).
            for &(q, r) in &coords {
                let mirrored = (-q, r + q);
                assert!(
                    coords.contains(&mirrored),
                    "shape of {nodes} nodes is not mirror-symmetric at ({q}, {r})"
                );
            }

            // Connectivity (BFS over the neighbor graph) and no isolated
            // nodes.
            let (built, _) = build_nodes(&coords);
            let mut reachable = vec![false; nodes];
            let mut stack = vec![0usize];
            reachable[0] = true;
            while let Some(id) = stack.pop() {
                for neighbor in built[id].neighbors.iter().flatten() {
                    if !reachable[*neighbor] {
                        reachable[*neighbor] = true;
                        stack.push(*neighbor);
                    }
                }
            }
            assert!(
                reachable.iter().all(|&seen| seen),
                "shape of {nodes} nodes is not connected"
            );
            for (id, node) in built.iter().enumerate() {
                assert!(
                    node.neighbors.iter().any(Option::is_some),
                    "node {id} of shape {nodes} has no in-board neighbor"
                );
            }
        }
    }

    #[test]
    fn neighbor_tables_are_antisymmetric() {
        for nodes in 8..=16 {
            let coords = symmetric_board(nodes);
            let (built, _) = build_nodes(&coords);
            for (id, node) in built.iter().enumerate() {
                for (i, &neighbor) in node.neighbors.iter().enumerate() {
                    if let Some(neighbor) = neighbor {
                        let back = built[neighbor].neighbors[Dir::ALL[i].opposite() as usize];
                        assert_eq!(back, Some(id));
                    }
                }
            }
        }
    }

    #[test]
    fn accessors_report_the_board() {
        let game = line_game();
        assert_eq!(game.node_count(), 3);
        assert_eq!(game.piece_count(), 2);
        assert_eq!(game.empty_count(), 1);
        assert_eq!(game.node_at(0, -1), Some(1));
        assert_eq!(game.node_at(5, 5), None);
        assert_eq!(game.piece_at(1), Some(0));
        assert_eq!(game.piece_at(2), None);
        assert_eq!(game.node_of(0), Some(1));
        assert_eq!(game.pieces()[1].arrows, Arrows::from_dir(Dir::Down));
        assert_eq!(game.nodes()[1].q, 0);
        assert_eq!(game.nodes()[1].r, -1);
    }

    #[test]
    fn starts_unsatisfied_and_reports_which_arrows() {
        let game = line_game();
        assert!(!game.is_solved());
        assert_eq!(game.status(), GameStatus::InProgress);
        // Piece 0 sits on node 1 at (0,-1); its Up arrow points off-board.
        assert_eq!(game.unsatisfied_arrows(0), Arrows::from_dir(Dir::Up));
        // Piece 1 sits on node 0; its Down arrow points at the empty node 2.
        assert_eq!(game.unsatisfied_arrows(1), Arrows::from_dir(Dir::Down));
        assert!(!game.satisfied(0));
        assert!(!game.satisfied(1));
    }

    #[test]
    fn illegal_moves_are_rejected() {
        let mut game = line_game();
        assert!(!game.can_move(0, 1)); // own node
        assert!(!game.can_move(0, 0)); // occupied by piece 1
        assert!(!game.can_move(0, 99)); // off-board
        assert_eq!(game.moves(), 0);
        assert!(!game.move_piece(0, 0));
        assert!(!game.move_piece(0, 1));
        assert_eq!(game.moves(), 0);
    }

    #[test]
    fn winning_move_latches_and_reset_restores() {
        let mut game = line_game();
        assert!(game.move_piece(0, 2));
        assert_eq!(game.moves(), 1);
        assert!(game.is_solved());
        assert_eq!(game.status(), GameStatus::Won);
        assert_eq!(game.unsatisfied_arrows(0), Arrows::default());
        assert!(game.satisfied(1));

        // The win is latched: no further moves are accepted.
        assert!(!game.can_move(1, 1));
        assert!(!game.move_piece(1, 1));
        assert_eq!(game.moves(), 1);

        game.reset();
        assert_eq!(game.status(), GameStatus::InProgress);
        assert_eq!(game.moves(), 0);
        assert_eq!(game.piece_at(1), Some(0));
        assert!(!game.is_solved());
    }

    #[test]
    fn params_are_validated() {
        let valid = Params {
            nodes: symmetric_board(8),
            pieces: 6,
            min_arrows: 2,
            max_arrows: 3,
        };
        validate_params(&valid);

        let mut dup = valid.clone();
        dup.nodes[1] = dup.nodes[0];
        assert!(std::panic::catch_unwind(|| validate_params(&dup)).is_err());

        let mut full = valid.clone();
        full.pieces = full.nodes.len();
        assert!(std::panic::catch_unwind(|| validate_params(&full)).is_err());

        let mut no_pieces = valid.clone();
        no_pieces.pieces = 0;
        assert!(std::panic::catch_unwind(|| validate_params(&no_pieces)).is_err());

        let mut no_arrows = valid.clone();
        no_arrows.min_arrows = 0;
        assert!(std::panic::catch_unwind(|| validate_params(&no_arrows)).is_err());

        let mut too_many = valid.clone();
        too_many.max_arrows = 7;
        assert!(std::panic::catch_unwind(|| validate_params(&too_many)).is_err());

        let mut inverted = valid.clone();
        inverted.min_arrows = 4;
        inverted.max_arrows = 3;
        assert!(std::panic::catch_unwind(|| validate_params(&inverted)).is_err());

        let mut isolated = valid.clone();
        isolated.nodes.push((100, 100));
        assert!(std::panic::catch_unwind(|| validate_params(&isolated)).is_err());
    }
}
