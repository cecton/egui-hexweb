#![doc = include_str!("../README.md")]

mod game;
mod generator;
mod solver;
mod widget;

pub use game::{
    hexagon, random_symmetric_board, Arrows, Dir, GameStatus, HexwebGame, Params, Piece,
};
pub use widget::{content_size, fit_cell_size, HexwebWidget};
