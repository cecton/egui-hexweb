//! Game logic for the hexweb puzzle: a hexagonal lattice of nodes where a
//! subset of nodes holds pieces, each piece carrying arrows that must all
//! point at occupied nodes.
//!
//! This module is pure logic: no `egui` types, no painting, no input. The
//! [`HexwebGame`] struct owns the board and the piece placement; the widget
//! in `crate::widget` renders and drives it.

use std::collections::{BTreeMap, BTreeSet};

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
    /// Axial coordinates of the board's nodes, e.g. from
    /// [`random_symmetric_board`].
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

/// An axial cell coordinate.
type Cell = (i32, i32);

/// One 60° rotation of the axial lattice.
fn rot((q, r): Cell) -> Cell {
    (-r, q + r)
}

/// The inverse of [`rot`]. Only the tests conjugate mirror maps with it.
#[cfg(test)]
fn rot_inv((q, r): Cell) -> Cell {
    (r + q, -q)
}

/// Reflection across the vertical pixel axis: the `q = 0` column stays put.
fn mirror_v((q, r): Cell) -> Cell {
    (-q, r + q)
}

/// Reflection across the horizontal pixel axis.
fn mirror_h((q, r): Cell) -> Cell {
    (q, -q - r)
}

/// An orbit of a mirror map over the board cells: either a single on-axis
/// cell or a mirror pair.
type Orbit = (Cell, Option<Cell>);

/// Splits `cells` (which must be closed under `mirror`) into its mirror
/// orbits, in the order the cells appear in the input.
fn mirror_orbits(cells: &[Cell], mirror: fn(Cell) -> Cell) -> Vec<Orbit> {
    let mut seen: BTreeSet<Cell> = BTreeSet::new();
    let mut orbits = Vec::new();
    for &cell in cells {
        if seen.insert(cell) {
            let image = mirror(cell);
            let twin = (image != cell).then_some(image);
            seen.extend(twin);
            orbits.push((cell, twin));
        }
    }
    orbits
}

/// Whether `coords` forms one connected piece under the lattice's six
/// neighbor directions.
fn is_connected(coords: &[Cell]) -> bool {
    let set: BTreeSet<Cell> = coords.iter().copied().collect();
    let start = coords[0];
    let mut seen: BTreeSet<Cell> = BTreeSet::from([start]);
    let mut stack = vec![start];
    while let Some(cell) = stack.pop() {
        for dir in Dir::ALL {
            let (dq, dr) = dir.delta();
            let next = (cell.0 + dq, cell.1 + dr);
            if set.contains(&next) && seen.insert(next) {
                stack.push(next);
            }
        }
    }
    seen.len() == coords.len()
}

/// Every mirror-symmetric connected board with exactly `nodes` nodes, in a
/// deterministic order.
///
/// The candidate shapes are the subsets of the 19-cell `hexagon(2)` board
/// that are closed under one of the lattice's six mirror axes: enumerating
/// the mirror orbits under the two axis classes (vertical and horizontal)
/// and rotating the results by 0/60/120 degrees covers all six. Subsets of
/// the wrong size or split into several pieces are dropped; pieces can only
/// move within a connected group, so a board of islands would freeze its
/// far pieces.
fn symmetric_candidates(nodes: usize) -> Vec<Vec<Cell>> {
    let pool = hexagon(2);
    let mut found: BTreeSet<Vec<Cell>> = BTreeSet::new();
    for mirror in [mirror_v, mirror_h] {
        let orbits = mirror_orbits(&pool, mirror);
        for mask in 0..1u64 << orbits.len() {
            let mut coords: Vec<Cell> = Vec::new();
            for (bit, &(cell, twin)) in orbits.iter().enumerate() {
                if mask >> bit & 1 == 1 {
                    coords.push(cell);
                    coords.extend(twin);
                }
            }
            if coords.len() != nodes || !is_connected(&coords) {
                continue;
            }
            // Connectivity is rotation-invariant, so checking the base
            // orientation is enough. Rotations by 180° (and, for shapes with
            // their own rotational symmetry, other overlapping rotations)
            // collapse into duplicates via the set below.
            for times in 0..3u32 {
                let mut candidate = coords.clone();
                for _ in 0..times {
                    candidate = candidate.iter().copied().map(rot).collect();
                }
                candidate.sort_unstable();
                found.insert(candidate);
            }
        }
    }
    found.into_iter().collect()
}

/// A mirror-symmetric board with exactly `nodes` nodes, for any count from 8
/// to 16, picked at random from every such shape.
///
/// Shapes may have holes and irregular outlines (there are dozens to
/// hundreds per node count), but they are always mirror-symmetric about one
/// of the six lattice axes and always one connected piece. `seed` picks one
/// of them uniformly; the same seed always yields the same shape.
///
/// Panics for node counts outside `8..=16`.
pub fn random_symmetric_board(nodes: usize, seed: u64) -> Vec<(i32, i32)> {
    assert!(
        (8..=16).contains(&nodes),
        "random_symmetric_board supports 8..=16 nodes, got {nodes}"
    );
    let candidates = symmetric_candidates(nodes);
    let mut rng = fastrand::Rng::with_seed(seed);
    candidates[rng.usize(..candidates.len())].clone()
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
    /// not `piece`'s own node. The other way a piece can land is onto an
    /// occupied node, swapping the two: see [`HexwebGame::can_swap`].
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

    /// Whether pieces `a` and `b` may be swapped right now: the game must be
    /// unfinished and both pieces must be placed and distinct. Two pieces
    /// with identical arrows may be swapped, but the swap is a free no-op
    /// (see [`HexwebGame::swap_pieces`]).
    pub fn can_swap(&self, a: PieceId, b: PieceId) -> bool {
        self.status == GameStatus::InProgress
            && a != b
            && self.node_of(a).is_some()
            && self.node_of(b).is_some()
    }

    /// Exchanges the nodes of the two placed pieces `a` and `b`. Returns
    /// whether the swap happened; illegal swaps (see
    /// [`HexwebGame::can_swap`]) leave the board unchanged. Counts the move
    /// and latches the win when the board becomes solved. Pieces with
    /// identical arrows are interchangeable, so swapping two of them leaves
    /// the move counter alone.
    pub fn swap_pieces(&mut self, a: PieceId, b: PieceId) -> bool {
        if !self.can_swap(a, b) {
            return false;
        }
        let from = self.node_of(a).unwrap();
        let to = self.node_of(b).unwrap();
        self.cell_piece[from] = Some(b);
        self.cell_piece[to] = Some(a);
        if self.pieces[a] != self.pieces[b] {
            self.moves += 1;
        }
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

    /// The mirror map about the axis obtained by rotating `mirror`'s axis by
    /// `k` steps of 60°.
    fn conjugate(mirror: fn(Cell) -> Cell, k: u32) -> impl Fn(Cell) -> Cell {
        move |cell: Cell| {
            let mut c = cell;
            for _ in 0..k {
                c = rot_inv(c);
            }
            let mut out = mirror(c);
            for _ in 0..k {
                out = rot(out);
            }
            out
        }
    }

    #[test]
    fn random_boards_are_symmetric_connected_and_sized() {
        let pool = hexagon(2);
        for nodes in 8..=16 {
            for seed in 0..32u64 {
                let coords = random_symmetric_board(nodes, seed);
                assert_eq!(coords.len(), nodes, "shape of {nodes} nodes, seed {seed}");
                assert!(
                    coords.windows(2).all(|w| w[0] < w[1]),
                    "shape of {nodes} nodes, seed {seed} is not sorted"
                );

                // Mirror invariance under at least one of the six axes.
                let set: BTreeSet<Cell> = coords.iter().copied().collect();
                let mut symmetric = false;
                for mirror in [mirror_v, mirror_h] {
                    for k in 0..3u32 {
                        let map = conjugate(mirror, k);
                        symmetric |= set.iter().all(|&c| set.contains(&map(c)));
                    }
                }
                assert!(
                    symmetric,
                    "shape of {nodes} nodes, seed {seed} is not mirror-symmetric"
                );

                assert!(
                    coords.iter().all(|c| pool.contains(c)),
                    "shape of {nodes} nodes, seed {seed} leaves hexagon(2)"
                );
                assert!(
                    is_connected(&coords),
                    "shape of {nodes} nodes, seed {seed} is not connected"
                );

                // Same seed, same shape.
                assert_eq!(coords, random_symmetric_board(nodes, seed));
            }
        }
    }

    #[test]
    fn every_size_offers_a_variety_of_shapes() {
        for nodes in 8..=16 {
            let candidates = symmetric_candidates(nodes);
            println!("{nodes} nodes: {} shapes", candidates.len());
            assert!(
                candidates.len() >= 10,
                "only {} shapes for {nodes} nodes",
                candidates.len()
            );
            let unique: BTreeSet<&Vec<Cell>> = candidates.iter().collect();
            assert_eq!(
                unique.len(),
                candidates.len(),
                "duplicate shapes at {nodes} nodes"
            );
        }

        // Known shapes stay reachable: the old 8-node board (7-node hexagon
        // plus one pendant) and the old 16-node board (hexagon(2) with a
        // three-cell bite out of one edge).
        let mut old8 = hexagon(1);
        old8.push((0, -2));
        old8.sort_unstable();
        assert!(symmetric_candidates(8).contains(&old8));
        let old16: Vec<Cell> = hexagon(2)
            .into_iter()
            .filter(|&c| !matches!(c, (0, 2) | (2, 0) | (-2, 2)))
            .collect();
        assert!(symmetric_candidates(16).contains(&old16));
    }

    #[test]
    fn neighbor_tables_are_antisymmetric() {
        for nodes in 8..=16 {
            let coords = random_symmetric_board(nodes, nodes as u64);
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

    /// A 4-node vertical line: node 0 at (0,0), node 1 at (0,-1), node 2 at
    /// (0,1), node 3 at (0,2). Piece 0 (arrow Up) sits on node 0, piece 1
    /// (arrow Down) on node 3, nodes 1 and 2 stay empty: piece 0 points at
    /// the empty node 1 and piece 1 points off-board, so the game is
    /// unsolved, and swapping the two is a legal move that doesn't solve it.
    fn long_line_game() -> HexwebGame {
        let (nodes, _) = build_nodes(&[(0, 0), (0, -1), (0, 1), (0, 2)]);
        let pieces = vec![
            Piece {
                arrows: Arrows::from_dir(Dir::Up),
            },
            Piece {
                arrows: Arrows::from_dir(Dir::Down),
            },
        ];
        let cell_piece = vec![Some(0), None, None, Some(1)];
        HexwebGame::from_parts(nodes, pieces, cell_piece.clone(), cell_piece)
    }

    /// Like [`long_line_game`], but both pieces carry the same single Up
    /// arrow, so the two are interchangeable.
    fn identical_pair_game() -> HexwebGame {
        let (nodes, _) = build_nodes(&[(0, 0), (0, -1), (0, 1), (0, 2)]);
        let pieces = vec![
            Piece {
                arrows: Arrows::from_dir(Dir::Up),
            },
            Piece {
                arrows: Arrows::from_dir(Dir::Up),
            },
        ];
        let cell_piece = vec![Some(0), None, None, Some(1)];
        HexwebGame::from_parts(nodes, pieces, cell_piece.clone(), cell_piece)
    }

    #[test]
    fn swapping_pieces_exchanges_nodes_and_counts_a_move() {
        let mut game = long_line_game();
        assert!(game.can_swap(0, 1));
        let (node_a, node_b) = (game.node_of(0), game.node_of(1));
        assert!(game.swap_pieces(0, 1));
        assert_eq!(game.node_of(0), node_b);
        assert_eq!(game.node_of(1), node_a);
        assert_eq!(game.moves(), 1);
        assert_eq!(game.status(), GameStatus::InProgress);
    }

    #[test]
    fn winning_swap_latches_and_reset_restores() {
        let mut game = line_game();
        // Swapping the two pieces solves it: each lands pointing at the
        // other.
        assert!(game.swap_pieces(0, 1));
        assert_eq!(game.moves(), 1);
        assert!(game.is_solved());
        assert_eq!(game.status(), GameStatus::Won);

        // The win is latched: no further swaps are accepted.
        assert!(!game.can_swap(0, 1));
        assert!(!game.swap_pieces(0, 1));
        assert_eq!(game.moves(), 1);

        game.reset();
        assert_eq!(game.status(), GameStatus::InProgress);
        assert_eq!(game.moves(), 0);
        assert_eq!(game.node_of(0), Some(1));
        assert_eq!(game.node_of(1), Some(0));
        assert!(!game.is_solved());
    }

    #[test]
    fn illegal_swaps_are_rejected() {
        let mut game = long_line_game();
        assert!(!game.can_swap(0, 0)); // same piece
        assert!(!game.can_swap(0, 7)); // no such piece
        assert!(!game.swap_pieces(0, 0));
        assert!(!game.swap_pieces(0, 7));
        assert_eq!(game.moves(), 0);
        assert_eq!(game.node_of(0), Some(0));
        assert_eq!(game.node_of(1), Some(3));
    }

    #[test]
    fn swapping_identical_pieces_does_not_count_a_move() {
        let mut game = identical_pair_game();
        assert!(game.can_swap(0, 1));
        let (node_a, node_b) = (game.node_of(0), game.node_of(1));
        assert!(game.swap_pieces(0, 1));
        assert_eq!(game.node_of(0), node_b);
        assert_eq!(game.node_of(1), node_a);
        // Interchangeable pieces: the position is unchanged, so the swap is
        // free.
        assert_eq!(game.moves(), 0);
        assert_eq!(game.status(), GameStatus::InProgress);
    }

    #[test]
    fn params_are_validated() {
        let valid = Params {
            nodes: random_symmetric_board(8, 0),
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
