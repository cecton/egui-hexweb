//! The egui widget that renders and drives a [`HexwebGame`]. This is the
//! only module allowed to touch `egui::Ui`/`Painter`.
//!
//! # Interaction model
//!
//! Pieces move by drag & drop, with a click-to-select fallback: press a
//! piece and release it over any empty node, or click a piece and then
//! click where it should go. While a piece is dragged it goes translucent
//! at its origin and every empty node grows a landing ring, the nearest
//! one emphasized; releasing anywhere that is not a legal target snaps the
//! piece back. A piece never leaves its node until the gesture commits, so
//! a drag that ends badly is not a move at all.

use std::f32::consts::TAU;

use egui::{
    Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Response, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2, Widget,
};

use crate::game::{Dir, GameStatus, HexwebGame, Node, NodeId, PieceId};

/// `sqrt(3)`: the vertical distance between two hexagon rows, as a
/// fraction of the circumradius.
const SQRT_3: f32 = 1.732_050_8;

/// Piece hexagon radius, as a fraction of the cell size. Slightly under 1.0
/// so a rim of the socket shows around every piece.
const PIECE_RADIUS: f32 = 0.88;
/// Chevron geometry, as fractions of the piece radius: how far from the
/// piece's centre the arrow tip sits, how far back its wings start, and how
/// far apart they are.
const ARROW_TIP: f32 = 0.62;
const ARROW_BACK: f32 = 0.30;
const ARROW_SPREAD: f32 = 0.17;
/// Arrow stroke width, as a fraction of the cell size.
const ARROW_STROKE: f32 = 0.085;
/// Radius of the landing and selection rings, as a fraction of the cell
/// size. Under the socket apothem (`sqrt(3)/2`) so rings stay inside the
/// node they mark.
const RING_RADIUS: f32 = 0.80;

/// The total footprint [`HexwebWidget`] will occupy for `game` at a given
/// `cell_size`. Derived from the actual node centers, so irregular board
/// shapes (a pendant node, a notched hexagon) get a tight box instead of a
/// bounding square. Lets a caller pre-size a container before laying the
/// widget out.
pub fn content_size(game: &HexwebGame, cell_size: f32) -> Vec2 {
    let (min, max) = center_bounds(game, cell_size);
    (max - min) + Vec2::new(2.0, SQRT_3) * cell_size
}

/// The cell size (hex circumradius, in logical pixels) that fits `game`'s
/// board into `available` space — the same auto-sizing formula
/// [`HexwebWidget`] uses internally when no explicit `cell_size` is set.
/// Lets a caller pre-compute that fit (e.g. to size a `Scene` before laying
/// the widget out inside it) instead of duplicating the formula.
pub fn fit_cell_size(game: &HexwebGame, available: Vec2) -> f32 {
    let unit = content_size(game, 1.0);
    (available.x / unit.x).min(available.y / unit.y).max(4.0)
}

/// Centre of node `(q, r)` relative to the `(0, 0)` centre, for a flat-top
/// hexagon of circumradius `size`: columns step `1.5·size` sideways, rows
/// `sqrt(3)·size` vertically, with every odd column shifted half a row.
fn node_center(q: i32, r: i32, size: f32) -> Vec2 {
    Vec2::new(
        1.5 * size * q as f32,
        SQRT_3 * size * (r as f32 + 0.5 * q as f32),
    )
}

/// Screen angle of a direction. `Dir::ALL` runs clockwise from the top, and
/// egui angles run clockwise from +x with y pointing down, so `Dir::Up` is
/// at -90°.
fn dir_angle(dir: Dir) -> f32 {
    -TAU / 4.0 + TAU / 6.0 * dir as u8 as f32
}

/// The (min, max) of the node centers laid out at circumradius `size`.
fn center_bounds(game: &HexwebGame, size: f32) -> (Vec2, Vec2) {
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    for node in game.nodes() {
        let c = node_center(node.q, node.r, size);
        min = min.min(c);
        max = max.max(c);
    }
    (min, max)
}

/// The corners of a flat-top hexagon, clockwise.
fn hex_corners(center: Pos2, radius: f32) -> Vec<Pos2> {
    (0..6)
        .map(|i| center + Vec2::angled(TAU / 6.0 * i as f32) * radius)
        .collect()
}

/// One arrow, as a chevron pointing from the piece's centre toward `dir`.
fn draw_arrow(painter: &egui::Painter, center: Pos2, dir: Dir, radius: f32, stroke: Stroke) {
    let v = Vec2::angled(dir_angle(dir));
    let tip = center + v * (radius * ARROW_TIP);
    let back = center + v * (radius * ARROW_BACK);
    let spread = v.rot90() * (radius * ARROW_SPREAD);
    painter.add(Shape::line(vec![back + spread, tip, back - spread], stroke));
}

/// The mapping between board nodes and screen positions for one widget
/// pass. Shared by painting and hit-testing so the two can never drift
/// apart.
#[derive(Clone, Copy)]
struct Geometry {
    /// Top-left of the widget's allocated rect.
    origin: Pos2,
    /// Hex circumradius.
    cell: f32,
    /// Node-center bounding-box min at this `cell`, used to shift the board
    /// into the allocated rect with a margin of one hexagon.
    min: Vec2,
}

impl Geometry {
    fn new(origin: Pos2, cell: f32, game: &HexwebGame) -> Self {
        Self {
            origin,
            cell,
            min: center_bounds(game, cell).0,
        }
    }

    /// Screen position of `node`'s center.
    fn center(&self, node: &Node) -> Pos2 {
        self.origin + node_center(node.q, node.r, self.cell) - self.min
            + Vec2::new(self.cell, SQRT_3 * 0.5 * self.cell)
    }

    /// The node nearest `pos`, if it is close enough to claim the pointer.
    /// Neighboring centers sit `sqrt(3)·cell` apart, so gating the nearest
    /// at one circumradius covers every node's own hexagonal cell without
    /// any two cells overlapping.
    fn node_at(&self, game: &HexwebGame, pos: Pos2) -> Option<NodeId> {
        let mut best = None;
        let mut best_d = f32::INFINITY;
        for (id, node) in game.nodes().iter().enumerate() {
            let d = pos.distance(self.center(node));
            if d < best_d {
                best = Some(id);
                best_d = d;
            }
        }
        match best {
            Some(id) if best_d <= self.cell => Some(id),
            _ => None,
        }
    }
}

/// An egui widget that renders an interactive hexweb board.
///
/// Drag a piece onto any empty node, or click a piece and then click its
/// destination. Arrows pointing at another piece are drawn in
/// `satisfied_color`; the ones still pointing at an empty node or off the
/// board are drawn in `unsatisfied_color`, which is the board's live
/// progress feedback. The puzzle is solved when no arrow is unsatisfied.
///
/// ```ignore
/// ui.add(egui_hexweb::HexwebWidget::new(&mut game));
/// ```
pub struct HexwebWidget<'a> {
    game: &'a mut HexwebGame,
    cell_size: Option<f32>,
    win_message: Option<String>,
    interactive: bool,
    satisfied_color: Option<Color32>,
    unsatisfied_color: Option<Color32>,
}

impl<'a> HexwebWidget<'a> {
    pub fn new(game: &'a mut HexwebGame) -> Self {
        Self {
            game,
            cell_size: None,
            win_message: None,
            interactive: true,
            satisfied_color: None,
            unsatisfied_color: None,
        }
    }

    /// Override the size (in logical pixels) of each hexagon's
    /// circumradius. When not set, the cell size is computed automatically
    /// to fill the available space of the parent container.
    pub fn cell_size(mut self, size: f32) -> Self {
        self.cell_size = Some(size);
        self
    }

    /// Message shown in the win banner drawn over the board once
    /// [`GameStatus::Won`] is reached. Defaults to `"Solved!"`. The crate
    /// ships no translations; embedding apps pass their own.
    pub fn win_message(mut self, message: impl Into<String>) -> Self {
        self.win_message = Some(message.into());
        self
    }

    /// Color for arrows that point at an occupied node. Defaults to the
    /// theme's strong text color.
    pub fn satisfied_color(mut self, color: Color32) -> Self {
        self.satisfied_color = Some(color);
        self
    }

    /// Color for arrows that point at an empty node or off the board.
    /// Defaults to the theme's error color — the accent the player reads
    /// as "this piece still needs to move".
    pub fn unsatisfied_color(mut self, color: Color32) -> Self {
        self.unsatisfied_color = Some(color);
        self
    }

    /// Whether the widget responds to input at all. Set to `false` to
    /// render the board read-only. Defaults to `true`.
    pub fn interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }
}

impl Widget for HexwebWidget<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let Self {
            game,
            cell_size,
            win_message,
            interactive,
            satisfied_color,
            unsatisfied_color,
        } = self;

        let cell = cell_size.unwrap_or_else(|| fit_cell_size(game, ui.available_size()));
        let total_size = content_size(game, cell);

        let sense = if interactive {
            Sense::click_and_drag()
        } else {
            Sense::hover()
        };
        let (response, painter) = ui.allocate_painter(total_size, sense);
        let geometry = Geometry::new(response.rect.min, cell, game);

        // ── Input ────────────────────────────────────────────────────────
        // The dragged (or selected) piece is stashed in egui's per-widget
        // temp memory so a gesture spans frames without the game holding
        // any presentation state.
        let drag_id = response.id.with("hexweb_drag_piece");
        let select_id = response.id.with("hexweb_selected_piece");
        let can_play = interactive && game.status() == GameStatus::InProgress;
        let pointer = response.interact_pointer_pos();
        let mut dragging: Option<PieceId> = ui.ctx().data(|d| d.get_temp(drag_id));

        if can_play {
            if response.clicked() {
                // A stationary press+release never fires `drag_started()`,
                // so the click path owns both selection and click-to-move.
                let hit = pointer.and_then(|pos| geometry.node_at(game, pos));
                let hit_piece = hit.and_then(|node| game.piece_at(node));
                if let Some(piece) = hit_piece {
                    let already =
                        ui.ctx().data(|d| d.get_temp::<PieceId>(select_id)) == Some(piece);
                    ui.ctx().data_mut(|d| {
                        if already {
                            d.remove_temp::<PieceId>(select_id);
                        } else {
                            d.insert_temp(select_id, piece);
                        }
                    });
                } else if let Some(node) = hit {
                    if let Some(selected) = ui.ctx().data(|d| d.get_temp::<PieceId>(select_id)) {
                        if game.move_piece(selected, node) {
                            ui.ctx().data_mut(|d| d.remove_temp::<PieceId>(select_id));
                        }
                    }
                } else {
                    ui.ctx().data_mut(|d| d.remove_temp::<PieceId>(select_id));
                }
            } else if response.drag_started() {
                // Hit-test where the press started, not where the pointer
                // is now: the first move event can already have crossed
                // into a neighboring node's cell, and grabbing whatever
                // piece happens to be under the *current* pointer would
                // steal the wrong piece on a fast flick.
                let grabbed = ui
                    .input(|i| i.pointer.press_origin())
                    .and_then(|pos| geometry.node_at(game, pos))
                    .and_then(|node| game.piece_at(node));
                if let Some(piece) = grabbed {
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(drag_id, piece);
                        d.remove_temp::<PieceId>(select_id);
                    });
                    dragging = Some(piece);
                }
            }
        }

        // Where the dragged piece would land right now, if anywhere.
        let drag_target = if can_play {
            dragging.and_then(|piece| {
                pointer
                    .and_then(|pos| geometry.node_at(game, pos))
                    .filter(|&node| game.can_move(piece, node))
            })
        } else {
            None
        };

        if response.drag_stopped() {
            if let Some(piece) = ui.ctx().data_mut(|d| d.remove_temp::<PieceId>(drag_id)) {
                if let Some(node) = pointer
                    .and_then(|pos| geometry.node_at(game, pos))
                    .filter(|&node| game.can_move(piece, node))
                {
                    game.move_piece(piece, node);
                }
            }
            dragging = None;
        }

        if can_play {
            if dragging.is_some() {
                ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            } else if let Some(node) = response
                .hover_pos()
                .and_then(|pos| geometry.node_at(game, pos))
            {
                if game.piece_at(node).is_some() {
                    ui.ctx().set_cursor_icon(CursorIcon::Grab);
                } else if ui
                    .ctx()
                    .data(|d| d.get_temp::<PieceId>(select_id).is_some())
                {
                    ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                }
            }
        }

        // ── Painting ─────────────────────────────────────────────────────
        let visuals = ui.visuals();
        let satisfied_color = satisfied_color.unwrap_or(visuals.strong_text_color());
        let unsatisfied_color = unsatisfied_color.unwrap_or(visuals.error_fg_color);

        let (piece_fill, piece_edge) = if visuals.dark_mode {
            (Color32::from_gray(0x5C), Color32::from_gray(0x82))
        } else {
            (Color32::from_gray(0xC9), Color32::from_gray(0x8E))
        };
        let socket_fill = visuals.extreme_bg_color;
        let socket_edge = visuals.widgets.noninteractive.bg_stroke.color;

        // Sockets first: every node shows one, pieces draw over them.
        for node in game.nodes() {
            painter.add(Shape::convex_polygon(
                hex_corners(geometry.center(node), cell),
                socket_fill,
                Stroke::new(1.0, socket_edge),
            ));
        }

        // Landing rings while a piece is in the air: a faint ring on every
        // empty node, a strong one on the node the piece would drop on.
        if can_play && dragging.is_some() {
            for (id, node) in game.nodes().iter().enumerate() {
                if game.piece_at(id).is_some() {
                    continue;
                }
                let stroke = if drag_target == Some(id) {
                    Stroke::new(cell * 0.10, satisfied_color)
                } else {
                    Stroke::new(cell * 0.05, piece_fill)
                };
                painter.circle_stroke(geometry.center(node), cell * RING_RADIUS, stroke);
            }
        }

        // Selection ring for the click-to-select fallback.
        if can_play && dragging.is_none() {
            if let Some(selected) = ui.ctx().data(|d| d.get_temp::<PieceId>(select_id)) {
                if let Some(node) = game.node_of(selected) {
                    painter.circle_stroke(
                        geometry.center(&game.nodes()[node]),
                        cell * RING_RADIUS,
                        Stroke::new(cell * 0.075, satisfied_color),
                    );
                }
            }
        }

        // Pieces and their arrows, on top of everything.
        for piece in 0..game.piece_count() {
            let Some(node) = game.node_of(piece) else {
                continue;
            };
            let center = geometry.center(&game.nodes()[node]);
            let in_hand = dragging == Some(piece);
            let alpha = if in_hand { 0.35 } else { 1.0 };
            let arrows = game.pieces()[piece].arrows;
            let unsatisfied = game.unsatisfied_arrows(piece);

            painter.add(Shape::convex_polygon(
                hex_corners(center, cell * PIECE_RADIUS),
                piece_fill.gamma_multiply(alpha),
                Stroke::new(cell * 0.05, piece_edge.gamma_multiply(alpha)),
            ));
            for dir in arrows.iter() {
                let color = if unsatisfied.contains(dir) {
                    unsatisfied_color
                } else {
                    satisfied_color
                };
                draw_arrow(
                    &painter,
                    center,
                    dir,
                    cell * PIECE_RADIUS,
                    Stroke::new(cell * ARROW_STROKE, color.gamma_multiply(alpha)),
                );
            }
        }

        // Win banner, drawn last so it sits on top of everything else.
        if game.status() == GameStatus::Won {
            let message = win_message.unwrap_or_else(|| "Solved!".to_owned());
            let font = FontId::proportional((cell * 0.6).clamp(14.0, 40.0));
            let galley = painter.layout_no_wrap(message, font, satisfied_color);
            // Hugging the top edge rather than centred: the fully satisfied
            // ring of arrows is the reward for solving the board, and a
            // banner in the middle of it covers exactly the part worth
            // looking at.
            let size = galley.size() + Vec2::new(cell, cell * 0.6);
            let banner = Rect::from_center_size(
                Pos2::new(
                    response.rect.center().x,
                    response.rect.min.y + size.y * 0.5 + cell * 0.15,
                ),
                size,
            );
            painter.rect_filled(banner, CornerRadius::same(6), visuals.window_fill);
            painter.rect_stroke(
                banner,
                CornerRadius::same(6),
                Stroke::new(1.0, satisfied_color),
                StrokeKind::Inside,
            );
            painter.galley(
                banner.center() - galley.size() * 0.5,
                galley,
                Color32::PLACEHOLDER,
            );
        }

        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{random_symmetric_board, Params};

    /// Beginner-shaped board: 8 nodes, 6 pieces, deterministic per seed.
    fn preset_game(nodes: usize, pieces: usize, seed: u64) -> HexwebGame {
        HexwebGame::random(
            Params {
                nodes: random_symmetric_board(nodes, seed),
                pieces,
                min_arrows: 2,
                max_arrows: 4,
            },
            seed,
        )
    }

    #[test]
    fn dir_angles_match_the_pixel_deltas() {
        for dir in Dir::ALL {
            let (dq, dr) = dir.delta();
            let pixel = Vec2::new(1.5 * dq as f32, SQRT_3 * (dr as f32 + 0.5 * dq as f32));
            let angled = Vec2::angled(dir_angle(dir));
            assert!(
                (pixel.normalized() - angled).length() < 1e-4,
                "{dir:?} paints toward {angled:?}, lattice delta is {pixel:?}"
            );
        }
    }

    #[test]
    fn content_size_and_fit_round_trip() {
        let game = preset_game(12, 9, 7);
        let size = content_size(&game, 10.0);
        assert!((fit_cell_size(&game, size) - 10.0).abs() < 1e-4);
        // The board is not a square: the fit is bounded by the tighter side.
        assert!(size.x != size.y);
        assert!(fit_cell_size(&game, Vec2::splat(20.0)) >= 4.0);
    }

    #[test]
    fn node_at_round_trips_through_every_center() {
        let game = preset_game(16, 12, 3);
        let geometry = Geometry::new(Pos2::ZERO, 20.0, &game);
        for (id, node) in game.nodes().iter().enumerate() {
            assert_eq!(geometry.node_at(&game, geometry.center(node)), Some(id));
            // A small offset stays within the node's cell.
            let near = geometry.center(node) + Vec2::new(6.0, -4.0);
            assert_eq!(geometry.node_at(&game, near), Some(id));
        }
        assert_eq!(
            geometry.node_at(&game, Pos2::new(-999.0, -999.0)),
            None,
            "far away from the board there is nothing to hit"
        );
    }

    const TEST_CELL: f32 = 24.0;

    /// Runs the widget in a real `egui` pass so clicks and drags are
    /// exercised end to end: the click/drag split, the temp-memory gesture
    /// state, and the drop logic only behave correctly through egui's own
    /// pointer machinery, which a plain method call can't check.
    struct Harness {
        ctx: egui::Context,
        game: HexwebGame,
        rect: Rect,
        time: f64,
    }

    impl Harness {
        fn new(game: HexwebGame) -> Self {
            let mut harness = Self {
                ctx: egui::Context::default(),
                game,
                rect: Rect::ZERO,
                time: 0.0,
            };
            // The first pass places the board.
            harness.pass(Vec::new());
            harness
        }

        fn pass(&mut self, events: Vec<egui::Event>) -> Rect {
            self.time += 1.0 / 60.0;
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                events,
                time: Some(self.time),
                ..Default::default()
            };
            let game = &mut self.game;
            let rect = std::cell::Cell::new(Rect::ZERO);
            let _ = self.ctx.run_ui(input, |ui| {
                let response = ui.add(HexwebWidget::new(game).cell_size(TEST_CELL));
                rect.set(response.rect);
            });
            self.rect = rect.get();
            self.rect
        }

        fn geometry(&self) -> Geometry {
            Geometry::new(self.rect.min, TEST_CELL, &self.game)
        }

        fn center(&self, node: NodeId) -> Pos2 {
            self.geometry().center(&self.game.nodes()[node])
        }

        fn press(&mut self, pos: Pos2) {
            self.pass(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ]);
        }

        fn drag(&mut self, pos: Pos2) {
            self.pass(vec![egui::Event::PointerMoved(pos)]);
        }

        fn release(&mut self, pos: Pos2) {
            self.pass(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ]);
        }

        fn click(&mut self, pos: Pos2) {
            self.press(pos);
            self.release(pos);
        }
    }

    fn empty_node(game: &HexwebGame) -> NodeId {
        (0..game.node_count())
            .find(|&node| game.piece_at(node).is_none())
            .expect("presets always leave empty nodes")
    }

    fn occupied_node(game: &HexwebGame) -> NodeId {
        (0..game.node_count())
            .find(|&node| game.piece_at(node).is_some())
            .expect("presets always have pieces")
    }

    #[test]
    fn click_piece_then_empty_node_moves_it() {
        let mut harness = Harness::new(preset_game(8, 6, 11));
        let from = occupied_node(&harness.game);
        let piece = harness.game.piece_at(from).unwrap();
        let target = empty_node(&harness.game);

        harness.click(harness.center(from));
        assert_eq!(harness.game.node_of(piece), Some(from), "select only");
        harness.click(harness.center(target));

        assert_eq!(harness.game.node_of(piece), Some(target));
        assert_eq!(harness.game.moves(), 1);
    }

    #[test]
    fn clicking_an_empty_node_without_a_selection_does_nothing() {
        let mut harness = Harness::new(preset_game(8, 6, 11));
        harness.click(harness.center(empty_node(&harness.game)));
        assert_eq!(harness.game.moves(), 0);
    }

    #[test]
    fn clicking_the_selected_piece_again_deselects_it() {
        let mut harness = Harness::new(preset_game(8, 6, 11));
        let from = occupied_node(&harness.game);
        harness.click(harness.center(from));
        harness.click(harness.center(from));
        harness.click(harness.center(empty_node(&harness.game)));
        assert_eq!(harness.game.moves(), 0);
    }

    #[test]
    fn dragging_a_piece_to_an_empty_node_moves_it() {
        let mut harness = Harness::new(preset_game(12, 9, 5));
        let from = occupied_node(&harness.game);
        let piece = harness.game.piece_at(from).unwrap();
        let target = empty_node(&harness.game);

        harness.press(harness.center(from));
        // Cross egui's drag radius so this becomes a drag, not a click.
        harness.drag(harness.center(from) + Vec2::new(16.0, 16.0));
        harness.drag(harness.center(target));
        harness.release(harness.center(target));

        assert_eq!(harness.game.node_of(piece), Some(target));
        assert_eq!(harness.game.moves(), 1);
    }

    #[test]
    fn dropping_off_the_board_snaps_back() {
        let mut harness = Harness::new(preset_game(12, 9, 5));
        let from = occupied_node(&harness.game);
        let piece = harness.game.piece_at(from).unwrap();

        harness.press(harness.center(from));
        harness.drag(harness.center(from) + Vec2::new(16.0, 16.0));
        harness.release(Pos2::new(-100.0, -100.0));

        assert_eq!(harness.game.node_of(piece), Some(from));
        assert_eq!(harness.game.moves(), 0);
    }

    #[test]
    fn dropping_on_an_occupied_node_snaps_back() {
        let mut harness = Harness::new(preset_game(12, 9, 5));
        let from = occupied_node(&harness.game);
        let piece = harness.game.piece_at(from).unwrap();
        let taken = (0..harness.game.node_count())
            .find(|&node| node != from && harness.game.piece_at(node).is_some())
            .expect("a second occupied node exists");

        harness.press(harness.center(from));
        harness.drag(harness.center(from) + Vec2::new(16.0, 16.0));
        harness.release(harness.center(taken));

        assert_eq!(harness.game.node_of(piece), Some(from));
        assert_eq!(harness.game.moves(), 0);
    }

    /// Plays the board's unique solution through legal moves. Pieces can
    /// sit on each other's targets, so cycles are broken by parking a
    /// piece on any node no solution needs; there is always at least one
    /// empty node to park on.
    fn play_solution(game: &mut HexwebGame) {
        let Some(solution) = game.solution() else {
            panic!("test boards always have a solution");
        };
        let mut placed: Vec<bool> = (0..game.piece_count())
            .map(|piece| game.node_of(piece) == Some(solution[piece]))
            .collect();
        while placed.iter().any(|&done| !done) {
            // An unplaced piece whose target is empty can move right away.
            let step = (0..game.piece_count())
                .filter(|&piece| !placed[piece])
                .find_map(|piece| {
                    game.piece_at(solution[piece])
                        .is_none()
                        .then_some((piece, solution[piece]))
                })
                .or_else(|| {
                    // Otherwise every unplaced target is occupied: park any
                    // unplaced piece on a node no piece targets.
                    let piece = (0..game.piece_count()).find(|&piece| !placed[piece])?;
                    let node =
                        (0..game.node_count()).find(|&node| game.piece_at(node).is_none())?;
                    Some((piece, node))
                })
                .expect("a solvable board is always finishable");
            let (piece, node) = step;
            assert!(game.move_piece(piece, node));
            placed[piece] = game.node_of(piece) == Some(solution[piece]);
        }
    }

    #[test]
    fn a_won_board_ignores_input() {
        let mut harness = Harness::new(preset_game(8, 6, 2));
        play_solution(&mut harness.game);
        assert_eq!(harness.game.status(), GameStatus::Won);
        let moves = harness.game.moves();

        let from = occupied_node(&harness.game);
        let target = empty_node(&harness.game);
        // Clicks no longer select or move...
        harness.click(harness.center(from));
        harness.click(harness.center(target));
        // ...and drags no longer pick anything up.
        harness.press(harness.center(from));
        harness.drag(harness.center(from) + Vec2::new(16.0, 16.0));
        harness.release(harness.center(target));

        assert_eq!(harness.game.moves(), moves);
    }
}
