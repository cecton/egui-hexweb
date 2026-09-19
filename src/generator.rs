//! Puzzle generation: solution first, verify uniqueness, repair, scramble.
//!
//! The pipeline, and why each step is safe:
//!
//! 1. **Pick the solved occupancy set**: which nodes hold pieces. A node is
//!    only usable if at least one of its neighbors is also occupied (its
//!    piece must be able to carry at least one arrow).
//! 2. **Assign arrows**: each piece gets `min_arrows..=max_arrows` distinct
//!    directions, every one of them pointing at an occupied node. The
//!    constructed configuration is therefore a solution *by construction* —
//!    a board from this module always has at least one solution.
//! 3. **Verify and repair**: ask the exact solver for a solution *other*
//!    than the constructed one. If there is none, the board is unique and
//!    we're done. If there is one, add an arrow. Adding an arrow is
//!    **monotone**: as long as the new arrow points at a node occupied by the
//!    constructed configuration, the constructed configuration stays valid
//!    (so the count can never reach zero), and a strictly stronger
//!    constraint can never *create* a solution (so the count can never
//!    increase). Every successful repair therefore strictly decreases the
//!    number of solutions and the loop terminates. Repairs that provably
//!    kill the fetched alternate solution are preferred, which usually ends
//!    the search in a step or two.
//! 4. **Scramble**: apply a random walk of `2 × pieces` legal moves starting
//!    from the constructed solution. Every walk state is reachable from the
//!    solution by legal play (moves are reversible), so the puzzle can never
//!    be scrambled into a dead end. The walk is redone unless the result is
//!    not solved, not solvable in a single move or swap, and has displaced
//!    at least half the pieces.
//!
//! If `ATTEMPTS` full constructions fail to produce a unique board (never
//! observed at the shipped sizes — the exhaustive solver makes uniqueness
//! cheap to reach), the candidate with the lowest solution count seen is
//! returned instead, mirroring "playable beats infinite loop".

use crate::game::{Arrows, Dir, Node, NodeId, Piece, PieceId};
use crate::solver;

/// How many full construction attempts to make before falling back to the
/// least-ambiguous candidate seen.
const ATTEMPTS: usize = 400;

/// How many arrows may be added to one candidate before giving up on it.
/// Higher than it first looks like it needs: burning a candidate throws
/// away all its accumulated repairs, so converging slowly beats restarting.
const MAX_REPAIRS: usize = 200;

/// How many times to re-roll the occupancy set per attempt.
const MAX_SET_ROLLS: usize = 100;

/// How many times to re-roll the scramble walk per board.
const MAX_SCRAMBLE_ROLLS: usize = 100;

/// Cap used when ranking fallback candidates; way above what a playable
/// board should ever reach, way below what full counting would cost on a
/// pathological one.
const RANK_CAP: usize = 1000;

pub(crate) struct Generated {
    pub arrows: Vec<Arrows>,
    /// The scrambled starting placement: piece per node.
    pub scramble: Vec<Option<PieceId>>,
}

struct Candidate {
    arrows: Vec<Arrows>,
    /// The node each piece occupies in the constructed solution.
    solution_nodes: Vec<NodeId>,
    /// Solution count after repairs (capped at [`RANK_CAP`]).
    rank: usize,
}

pub(crate) fn generate(
    nodes: &[Node],
    pieces: usize,
    min_arrows: usize,
    max_arrows: usize,
    rng: &mut fastrand::Rng,
) -> Generated {
    let mut best: Option<Candidate> = None;
    for _ in 0..ATTEMPTS {
        let Some(solution_nodes) = pick_occupancy(nodes, pieces, rng) else {
            continue;
        };
        let mut arrows = assign_arrows(nodes, &solution_nodes, min_arrows, max_arrows, rng);
        let rank = repair_until_unique(nodes, &solution_nodes, &mut arrows, rng);
        if rank == 1 {
            return finish(
                nodes,
                Candidate {
                    arrows,
                    solution_nodes,
                    rank,
                },
                rng,
            );
        }
        if best.as_ref().is_none_or(|best| rank < best.rank) {
            best = Some(Candidate {
                arrows,
                solution_nodes,
                rank,
            });
        }
    }
    let best = best.expect("every occupancy roll failed; board cannot host pieces");
    finish(nodes, best, rng)
}

fn finish(nodes: &[Node], candidate: Candidate, rng: &mut fastrand::Rng) -> Generated {
    Generated {
        arrows: candidate.arrows.clone(),
        scramble: scramble(nodes, &candidate.arrows, &candidate.solution_nodes, rng),
    }
}

/// Picks the occupied set: `pieces` distinct nodes such that every one of
/// them has at least one occupied neighbor (otherwise its piece could carry
/// no arrow).
fn pick_occupancy(nodes: &[Node], pieces: usize, rng: &mut fastrand::Rng) -> Option<Vec<NodeId>> {
    let n = nodes.len();
    for _ in 0..MAX_SET_ROLLS {
        let mut ids: Vec<NodeId> = (0..n).collect();
        for i in 0..pieces {
            let j = rng.usize(i..n);
            ids.swap(i, j);
        }
        let mut mask = 0u64;
        for &id in &ids[..pieces] {
            mask |= 1 << id;
        }
        let all_supported = ids[..pieces].iter().all(|&id| {
            nodes[id]
                .neighbors
                .iter()
                .flatten()
                .any(|&neighbor| mask >> neighbor & 1 == 1)
        });
        if all_supported {
            return Some(ids[..pieces].to_vec());
        }
    }
    None
}

/// One arrow set per piece (in `solution_nodes` order): between `min_arrows`
/// and `max_arrows` distinct directions, all pointing at occupied nodes, so
/// the constructed configuration solves the board by construction.
fn assign_arrows(
    nodes: &[Node],
    solution_nodes: &[NodeId],
    min_arrows: usize,
    max_arrows: usize,
    rng: &mut fastrand::Rng,
) -> Vec<Arrows> {
    solution_nodes
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
        .collect()
}

/// Adds arrows until the constructed configuration is the only solution.
/// Returns the final solution count (capped at [`RANK_CAP`]); 1 means the
/// candidate is done.
///
/// Repairs are batched: against each fetched alternate solution, up to one
/// killer arrow per piece is added at once. Every arrow in a batch is on its
/// own a monotone repair (it keeps the constructed solution valid and kills
/// the alternate), so batching is as safe as single repairs while cutting
/// the number of solver sweeps by roughly the piece count.
fn repair_until_unique(
    nodes: &[Node],
    solution_nodes: &[NodeId],
    arrows: &mut [Arrows],
    rng: &mut fastrand::Rng,
) -> usize {
    let pieces_of = |arrows: &[Arrows]| -> Vec<Piece> {
        arrows.iter().map(|&arrows| Piece { arrows }).collect()
    };

    let mut repairs = 0;
    loop {
        let pieces = pieces_of(arrows);
        let Some(alternate) = solver::alternate_solution(&pieces, nodes, solution_nodes) else {
            // The constructed configuration is the only solution.
            return solver::count(&pieces, nodes, RANK_CAP);
        };
        if repairs == MAX_REPAIRS {
            return solver::count(&pieces, nodes, RANK_CAP);
        }

        let alt_occupied: u64 = alternate.iter().fold(0u64, |mask, &node| mask | 1 << node);
        let mut added = 0;
        for piece in 0..arrows.len() {
            if repairs == MAX_REPAIRS {
                break;
            }
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
                // Does the arrow break the fetched alternate placement of
                // this piece? If its node has no occupied node in that
                // direction, the alternate solution dies.
                let kills = match nodes[alternate[piece]].neighbors[dir as usize] {
                    Some(neighbor) => alt_occupied >> neighbor & 1 == 0,
                    None => true,
                };
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
                repairs += 1;
                added += 1;
            }
        }
        if added == 0 {
            // No arrow can be added while keeping the constructed solution
            // valid: this candidate is stuck, burn its remaining budget.
            return solver::count(&pieces_of(arrows), nodes, RANK_CAP);
        }
    }
}

/// Random walk of `2 × pieces` legal moves from the constructed solution,
/// re-rolled until the result is neither solved, nor one move away from
/// solved, nor keeps more than half the pieces in their solution nodes.
fn scramble(
    nodes: &[Node],
    arrows: &[Arrows],
    solution_nodes: &[NodeId],
    rng: &mut fastrand::Rng,
) -> Vec<Option<PieceId>> {
    let piece_count = solution_nodes.len();
    for _ in 0..MAX_SCRAMBLE_ROLLS {
        let mut cell: Vec<Option<PieceId>> = vec![None; nodes.len()];
        for (piece, &node) in solution_nodes.iter().enumerate() {
            cell[node] = Some(piece as PieceId);
        }
        for _ in 0..2 * piece_count {
            let occupied: Vec<(PieceId, NodeId)> = cell
                .iter()
                .enumerate()
                .filter_map(|(node, &occupant)| occupant.map(|piece| (piece, node)))
                .collect();
            let empties: Vec<NodeId> = (0..nodes.len())
                .filter(|&node| cell[node].is_none())
                .collect();
            if occupied.is_empty() || empties.is_empty() {
                break;
            }
            let (piece, from) = occupied[rng.usize(..occupied.len())];
            let to = empties[rng.usize(..empties.len())];
            cell[from] = None;
            cell[to] = Some(piece);
        }
        if is_good_scramble(nodes, arrows, &cell, solution_nodes) {
            return cell;
        }
    }
    // statistical near-impossibility: keep the last (imperfect) walk
    // rather than panicking on a playable-but-easy board.
    let mut cell: Vec<Option<PieceId>> = vec![None; nodes.len()];
    for (piece, &node) in solution_nodes.iter().enumerate() {
        cell[node] = Some(piece as PieceId);
    }
    cell
}

fn is_good_scramble(
    nodes: &[Node],
    arrows: &[Arrows],
    cell: &[Option<PieceId>],
    solution_nodes: &[NodeId],
) -> bool {
    if is_solved_placement(nodes, arrows, cell) {
        return false;
    }
    if single_move_solves(nodes, arrows, cell) {
        return false;
    }
    if single_swap_solves(nodes, arrows, cell) {
        return false;
    }
    // At least half the pieces must have left their solution node (with
    // identical pieces this is approximate, which is fine for an
    // anti-triviality heuristic).
    let placed_right = (0..solution_nodes.len())
        .filter(|&piece| cell[solution_nodes[piece]] == Some(piece as PieceId))
        .count();
    2 * placed_right <= solution_nodes.len()
}

fn is_solved_placement(nodes: &[Node], arrows: &[Arrows], cell: &[Option<PieceId>]) -> bool {
    cell.iter().enumerate().all(|(node, &occupant)| {
        let Some(piece) = occupant else {
            return true;
        };
        arrows[piece].iter().all(|dir| {
            nodes[node].neighbors[dir as usize].is_some_and(|neighbor| cell[neighbor].is_some())
        })
    })
}

fn single_move_solves(nodes: &[Node], arrows: &[Arrows], cell: &[Option<PieceId>]) -> bool {
    for from in 0..cell.len() {
        let Some(piece) = cell[from] else {
            continue;
        };
        for to in 0..cell.len() {
            if cell[to].is_some() {
                continue;
            }
            let mut moved = cell.to_vec();
            moved[from] = None;
            moved[to] = Some(piece);
            if is_solved_placement(nodes, arrows, &moved) {
                return true;
            }
        }
    }
    false
}

/// Whether swapping the occupants of two nodes would solve the placement.
/// Two identical pieces trade places for nothing, so those pairs can't
/// newly solve anything and are skipped.
fn single_swap_solves(nodes: &[Node], arrows: &[Arrows], cell: &[Option<PieceId>]) -> bool {
    for a in 0..cell.len() {
        let Some(piece_a) = cell[a] else {
            continue;
        };
        for b in (a + 1)..cell.len() {
            let Some(piece_b) = cell[b] else {
                continue;
            };
            if arrows[piece_a] == arrows[piece_b] {
                continue;
            }
            let mut swapped = cell.to_vec();
            swapped[a] = Some(piece_b);
            swapped[b] = Some(piece_a);
            if is_solved_placement(nodes, arrows, &swapped) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{random_symmetric_board, HexwebGame, Params};

    /// The three shipped presets: (nodes, pieces, min_arrows, max_arrows).
    fn presets() -> Vec<(usize, usize, usize, usize)> {
        vec![(8, 6, 2, 3), (12, 9, 2, 4), (16, 12, 3, 5)]
    }

    #[test]
    fn presets_generate_unique_unsolved_boards() {
        for (nodes, pieces, min_arrows, max_arrows) in presets() {
            for seed in 0..60u64 {
                let game = HexwebGame::random(
                    Params {
                        nodes: random_symmetric_board(nodes, seed),
                        pieces,
                        min_arrows,
                        max_arrows,
                    },
                    seed,
                );
                assert_eq!(game.piece_count(), pieces, "nodes={nodes} seed={seed}");
                assert_eq!(game.solution_count(4), 1, "nodes={nodes} seed={seed}");
                assert_eq!(game.status(), crate::game::GameStatus::InProgress);
                assert!(!game.is_solved(), "nodes={nodes} seed={seed}");
                assert!(
                    !single_move_solves(game.nodes(), &game_pieces(&game), &scramble_of(&game)),
                    "one move from victory at nodes={nodes} seed={seed}"
                );
                assert!(
                    !single_swap_solves(game.nodes(), &game_pieces(&game), &scramble_of(&game)),
                    "one swap from victory at nodes={nodes} seed={seed}"
                );
                for piece in 0..game.piece_count() {
                    assert!(
                        !game.pieces()[piece].arrows.is_empty(),
                        "arrowless piece at nodes={nodes} seed={seed}"
                    );
                }
            }
        }
    }

    #[test]
    fn generated_boards_are_winnable_by_legal_play() {
        for (nodes, pieces, min_arrows, max_arrows) in presets() {
            for seed in 0..5u64 {
                let mut game = HexwebGame::random(
                    Params {
                        nodes: random_symmetric_board(nodes, seed),
                        pieces,
                        min_arrows,
                        max_arrows,
                    },
                    seed,
                );
                let solution = game.solution().expect("generated boards are solvable");
                // Place pieces in order, using an empty node as buffer when
                // a destination is blocked (any permutation is reachable
                // because the board always has at least one empty node).
                for (piece, &dest) in solution.iter().enumerate() {
                    if game.node_of(piece) == Some(dest) {
                        continue;
                    }
                    if let Some(blocker) = game.piece_at(dest) {
                        let buffer = (0..game.node_count())
                            .find(|&node| game.piece_at(node).is_none())
                            .expect("generated boards always have an empty node");
                        assert!(game.move_piece(blocker, buffer));
                    }
                    assert!(game.move_piece(piece, dest));
                }
                assert_eq!(
                    game.status(),
                    crate::game::GameStatus::Won,
                    "nodes={nodes} seed={seed}"
                );
            }
        }
    }

    #[test]
    fn generation_is_reproducible() {
        for (nodes, pieces, min_arrows, max_arrows) in presets() {
            let params = || Params {
                nodes: random_symmetric_board(nodes, 0),
                pieces,
                min_arrows,
                max_arrows,
            };
            let a = HexwebGame::random(params(), 12345);
            let b = HexwebGame::random(params(), 12345);
            assert_eq!(format!("{a:?}"), format!("{b:?}"));
            let c = HexwebGame::random(params(), 12346);
            assert_ne!(format!("{a:?}"), format!("{c:?}"));
        }
    }

    #[test]
    fn reset_returns_to_the_scramble() {
        let mut game = HexwebGame::random(
            Params {
                nodes: random_symmetric_board(12, 0),
                pieces: 9,
                min_arrows: 2,
                max_arrows: 4,
            },
            7,
        );
        let initial: Vec<Option<_>> = (0..game.node_count())
            .map(|node| game.piece_at(node))
            .collect();
        // Make some legal moves, then undo them via reset.
        let mut moved_any = false;
        for piece in 0..game.piece_count() {
            for node in 0..game.node_count() {
                if game.can_move(piece, node) {
                    assert!(game.move_piece(piece, node));
                    moved_any = true;
                    break;
                }
            }
            if moved_any {
                break;
            }
        }
        assert!(moved_any);
        assert!(game.moves() > 0);
        game.reset();
        let restored: Vec<Option<_>> = (0..game.node_count())
            .map(|node| game.piece_at(node))
            .collect();
        assert_eq!(initial, restored);
        assert_eq!(game.moves(), 0);
        assert_eq!(game.status(), crate::game::GameStatus::InProgress);
    }

    fn game_pieces(game: &HexwebGame) -> Vec<Arrows> {
        game.pieces().iter().map(|piece| piece.arrows).collect()
    }

    fn scramble_of(game: &HexwebGame) -> Vec<Option<PieceId>> {
        (0..game.node_count())
            .map(|node| game.piece_at(node))
            .collect()
    }

    /// A larger-sample survey for freezing the difficulty table: uniqueness
    /// rate, generation cost, and average arrows per piece. Run with:
    ///
    /// ```sh
    /// cargo test --lib --release -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "offline survey: too slow for the normal suite"]
    fn difficulty_survey() {
        for (nodes, pieces, min_arrows, max_arrows) in presets() {
            let start = std::time::Instant::now();
            let mut unique = 0;
            let mut total_arrows = 0usize;
            let samples = 1000;
            for seed in 0..samples as u64 {
                let game = HexwebGame::random(
                    Params {
                        nodes: random_symmetric_board(nodes, seed),
                        pieces,
                        min_arrows,
                        max_arrows,
                    },
                    seed,
                );
                if game.solution_count(2) == 1 {
                    unique += 1;
                }
                total_arrows += game.pieces().iter().map(|p| p.arrows.len()).sum::<usize>();
            }
            println!(
                "nodes={nodes}: {unique}/{samples} unique, avg arrows/piece {:.2}, {:?} total",
                total_arrows as f64 / (samples * pieces) as f64,
                start.elapsed()
            );
            assert_eq!(unique, samples, "non-unique boards at nodes={nodes}");
        }
    }
}
