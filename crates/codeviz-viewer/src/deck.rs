//! Call order as a deck of cards, with time on the depth axis. Each call the disclosed
//! journey makes is a card: a faint copy of the map's footprint that draws only that
//! call, its caller and callee where the map has them and the call between them. The
//! cards stack in the order the calls run; the cursor's card lies on the map itself, so
//! the map never moves, and the calls either side of it step away from it, fading the
//! further they are in time. A function called twice is on two cards.
//!
//! The deck shows two ways, cycled with `Z`:
//! - fanned, like a hand of cards: head-on, each card a step right, down and back from
//!   the one before, so the calls read left to right as the journey's bars do;
//! - stacked: each card a step back along depth, seen from a gentle angle.
//!
//! Stepping through the journey leafs through the deck.

use bevy::prelude::*;
use bevy_egui::egui;
use codeviz_core::SpanId;

use crate::{
    camera::{ANGLED, FRONT, OrbitCamera},
    diff_view::DiffView,
    explorer::Explorer,
    graph_view::{self, node_color, node_size},
    scene::{PALETTE, lane_color},
    ui::{self, Chrome},
};

/// How the journey's order shows in depth.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TimeMode {
    #[default]
    Off,
    /// Head-on, the cards stepping diagonally like a fanned hand.
    Fan,
    /// The cards one behind another along depth, seen at a gentle angle.
    Stack,
}

impl TimeMode {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Off => Self::Fan,
            Self::Fan => Self::Stack,
            Self::Stack => Self::Off,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Fan => "fan",
            Self::Stack => "stack",
        }
    }

    /// The angle the camera turns to: the fan is seen head-on, so the map is too.
    fn view(self) -> (f32, f32) {
        match self {
            Self::Off => ANGLED,
            Self::Fan => FRONT,
            Self::Stack => STACK_VIEW,
        }
    }
}

/// Gentle, so the map keeps its shape while the stack spreads out from it.
const STACK_VIEW: (f32, f32) = (-0.32, -0.14);

/// How dark the veil over the map is while the deck shows.
const VEIL: u8 = 90;

/// Cards either side of the cursor's that draw.
pub(crate) const HALF: usize = 5;
/// Cards either side of the cursor's whose calls are named; further ones show only
/// their numbers.
const NAMED: f32 = 2.0;

/// One call on the journey, as its card draws it.
#[derive(Debug, Clone)]
pub(crate) struct Card {
    pub(crate) caller: usize,
    pub(crate) callee: usize,
    pub(crate) span: SpanId,
    pub(crate) number: String,
}

/// The deck as it stands: its cards in the order the calls run, where the cursor is
/// among them, eased, and how far one card sits from the next.
#[derive(Resource, Debug, Default)]
pub(crate) struct Deck {
    mode: TimeMode,
    pub(crate) cards: Vec<Card>,
    /// The cursor's place, in cards: its card's index, `-1` before the first call.
    at: Option<f32>,
    /// One card's step from the next, in the world.
    step: Vec3,
    /// The footprint's corners on the map plane: the journey's extent, with room.
    frame: Option<(Vec3, Vec3)>,
}

pub(crate) struct DeckPlugin;

impl Plugin for DeckPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Deck>();
    }
}

/// Each call `root`'s tree makes, as `(caller, callee)`, in the order they run: depth
/// first, a call's own calls before the next call its caller makes.
pub(crate) fn calls_in_order<S: Copy>(root: S, children: impl Fn(S) -> Vec<S>) -> Vec<(S, S)> {
    let mut calls = Vec::new();
    let mut stack: Vec<(S, S)> = children(root)
        .into_iter()
        .rev()
        .map(|c| (root, c))
        .collect();
    while let Some((caller, call)) = stack.pop() {
        calls.push((caller, call));
        stack.extend(children(call).into_iter().rev().map(|c| (call, c)));
    }
    calls
}

/// Where the cursor sits among `calls`: at its own call's card, else at the nearest
/// call above it that has one, else `-1`, before the first.
pub(crate) fn cursor_place<S: Copy + Eq>(
    calls: &[S],
    selected: Option<S>,
    parent: impl Fn(S) -> Option<S>,
) -> f32 {
    std::iter::successors(selected, |&s| parent(s))
        .find_map(|s| calls.iter().position(|&c| c == s))
        .map_or(-1.0, |index| {
            #[expect(clippy::cast_precision_loss, reason = "a card index")]
            let index = index as f32;
            index
        })
}

/// How strongly a card `away` cards in time from the cursor's draws: in full at the
/// cursor, a step down next to it, then fading with each card further, never quite
/// gone.
pub(crate) fn strength(away: f32) -> f32 {
    let away = away.abs();
    if away < 1.0 {
        1.0 + (NEXT - 1.0) * away
    } else {
        NEXT * (-0.3 * (away - 1.0)).exp()
    }
}

/// How strongly the cards next to the cursor's draw.
const NEXT: f32 = 0.62;

/// How far a card `away` cards from the cursor's sits from it, `step` being one card
/// on: fanned, each later card a step right and a little up, clear of the journey
/// docked below, and back, so the cursor's is the nearest; stacked, straight back along depth, the past in front.
pub(crate) fn offset(mode: TimeMode, away: f32, step: Vec3) -> Vec3 {
    match mode {
        TimeMode::Off => Vec3::ZERO,
        TimeMode::Fan => Vec3::new(away * step.x, away * step.y, -away.abs() * step.z),
        TimeMode::Stack => Vec3::new(0.0, 0.0, -away * step.z),
    }
}

/// The disclosed journey's calls, as cards, in the order they run; a call whose caller
/// or callee the graph lacks has no card.
fn cards(explorer: &Explorer) -> Vec<Card> {
    let Some(journey) = explorer.journey() else {
        return Vec::new();
    };
    let disclosed = explorer.disclosure();
    let map = explorer.map();
    let tile = |span: SpanId| explorer.unit_id(span).and_then(|u| map.tile_of(u));
    let shown = |span: SpanId| -> Vec<SpanId> {
        disclosed
            .children(span)
            .iter()
            .copied()
            .filter(|&c| disclosed.number(c).is_some())
            .collect()
    };
    calls_in_order(journey.root(), shown)
        .into_iter()
        .filter_map(|(caller, call)| {
            Some(Card {
                caller: tile(caller)?,
                callee: tile(call)?,
                span: call,
                number: disclosed.number(call)?.to_owned(),
            })
        })
        .collect()
}

/// Keeps the deck current with the journey and eases the cursor's place in it.
pub(crate) fn shuffle(
    time: Res<Time>,
    chrome: Res<Chrome>,
    explorer: Res<Explorer>,
    mut deck: ResMut<Deck>,
    cameras: Query<(&OrbitCamera, &Projection)>,
) {
    if chrome.time == TimeMode::Off {
        *deck = Deck::default();
        return;
    }
    deck.mode = chrome.time;
    deck.cards = cards(&explorer);
    let spans: Vec<SpanId> = deck.cards.iter().map(|c| c.span).collect();
    let target = cursor_place(&spans, explorer.selected(), |s| {
        explorer.span(s).and_then(|s| s.parent)
    });
    let ease = 1.0 - (-time.delta_secs() * 7.0).exp();
    deck.at = Some(deck.at.map_or(target, |at| at + (target - at) * ease));
    // The footprint: what the cards draw, with room for the rings round it.
    let points: Vec<Vec3> = deck
        .cards
        .iter()
        .flat_map(|c| [c.caller, c.callee])
        .filter_map(|t| explorer.force_point(t))
        .collect();
    deck.frame = footprint(&points);
    // A card's step is a share of the view, so the deck spans about the same part of
    // the screen on a small graph and a large one.
    let width = cameras.iter().next().map_or(100.0, |(camera, projection)| {
        let (fov, aspect) = match projection {
            Projection::Perspective(p) => (p.fov, p.aspect_ratio),
            _ => (std::f32::consts::FRAC_PI_4, 1.6),
        };
        2.0 * (fov / 2.0).tan() * aspect * camera.distance
    });
    deck.step = Vec3::new(width * 0.07, width * 0.012, width * 0.05);
}

/// A card's corners on the map plane: round most of `points`, with room, so a call
/// out to a far corner of the map does not stretch every card.
pub(crate) fn footprint(points: &[Vec3]) -> Option<(Vec3, Vec3)> {
    if points.is_empty() {
        return None;
    }
    let spread = |axis: fn(&Vec3) -> f32| {
        let mut values: Vec<f32> = points.iter().map(axis).collect();
        values.sort_by(f32::total_cmp);
        let last = values.len() - 1;
        (values[last / 10], values[last - last / 10])
    };
    let ((left, right), (bottom, top)) = (spread(|p| p.x), spread(|p| p.y));
    let depth = points.iter().map(|p| p.z).sum::<f32>() / points.len() as f32;
    let room = 6.0;
    Some((
        Vec3::new(left - room, bottom - room, depth),
        Vec3::new(right + room, top + room, depth),
    ))
}

/// Turns the camera to the deck's view as it comes and goes.
pub(crate) fn turn(
    chrome: Res<Chrome>,
    mut last: Local<TimeMode>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    if chrome.time == *last {
        return;
    }
    *last = chrome.time;
    for mut camera in &mut cameras {
        camera.turn_to(chrome.time.view());
    }
}

/// How strongly the map's own links and rings draw: back while the deck shows, so its
/// cards read over them.
pub(crate) fn map_strength(chrome: &Chrome) -> f32 {
    if chrome.time == TimeMode::Off {
        1.0
    } else {
        0.5
    }
}

/// What the deck left on screen: whether it shows, and where its names went, for the
/// map's names to keep off.
#[derive(Debug, Default)]
pub(crate) struct Dealt {
    pub(crate) showing: bool,
    pub(crate) names: Vec<egui::Rect>,
}

/// Draws the deck, furthest from the cursor first, so the cursor's card is on top.
pub(crate) fn draw(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    explorer: &Explorer,
    diff: Option<&DiffView>,
    deck: &Deck,
) -> Dealt {
    let mut taken = Vec::new();
    let mode = deck.mode;
    let Some(at) = deck.at.filter(|_| mode != TimeMode::Off) else {
        return Dealt::default();
    };
    #[expect(clippy::cast_precision_loss, reason = "card indices")]
    let mut order: Vec<(usize, f32)> = deck
        .cards
        .iter()
        .enumerate()
        .map(|(index, _)| (index, index as f32 - at))
        .filter(|&(_, away)| away.abs() <= HALF as f32 + 0.5)
        .collect();
    order.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
    // A veil over the map, so the cards read over it; its shape still shows through.
    painter.rect_filled(
        painter.clip_rect(),
        0.0,
        egui::Color32::from_black_alpha(VEIL),
    );
    let regions = graph_view::regions(explorer);
    draw_thread(painter, project, explorer, deck, mode, &order);
    draw_ends(painter, project, deck, mode, &order);
    // Each card's call as it lands on screen, for its names after.
    let mut dealt: Vec<(usize, f32, Option<Ends>)> = Vec::new();
    for &(index, away) in &order {
        let card = &deck.cards[index];
        let shift = offset(mode, away, deck.step);
        let s = strength(away);
        let on_map = away.abs() < 0.5;
        // The cursor's card is the map itself: only its outline, so it reads as the
        // top of the deck.
        let modules = if on_map {
            [None, None]
        } else {
            [card.caller, card.callee].map(|t| explorer.graph().cluster(t))
        };
        let outline = if on_map { 0.6 } else { s };
        draw_footprint(painter, project, deck, &regions, &modules, shift, outline);
        let ends = ends(explorer, project, card, shift);
        if let Some(ends) = ends {
            let line = egui::Color32::from_white_alpha(alpha(if on_map { 1.0 } else { 0.85 * s }));
            let width = if on_map { 3.0 } else { 1.2 + 1.3 * s };
            arrow(painter, ends, line, width);
            if !on_map {
                for (tile, (center, r)) in [(card.caller, ends.0), (card.callee, ends.1)] {
                    let fill = ui::egui_color(node_color(explorer, diff, tile));
                    disc(painter, center, r, fill, s);
                }
            }
        }
        dealt.push((index, away, ends));
    }
    // Names nearest the cursor first, so they win; the map names the cursor's own.
    for &(index, away, ends) in dealt.iter().rev() {
        let (Some(((a, ra), (b, rb))), true) = (ends, away.abs() >= 0.5) else {
            continue;
        };
        let card = &deck.cards[index];
        let s = strength(away);
        // The callee by its number, as its bar shows it, and near the cursor by name
        // too; the caller only where it is not the same as on the card before, so a
        // run of calls from one function names it once.
        let fill = ui::egui_color(node_color(explorer, diff, card.callee));
        let badge = Some((card.number.clone(), fill));
        let near = away.abs() < NAMED + 0.5;
        let name = if near {
            ui::node_name(explorer, card.callee)
        } else {
            String::new()
        };
        label(painter, &mut taken, b, rb, badge, &name, s);
        let before = index
            .checked_sub(1)
            .filter(|&b| dealt.iter().any(|d| d.0 == b));
        if near && before.is_none_or(|b| deck.cards[b].caller != card.caller) {
            let name = ui::node_name(explorer, card.caller);
            label(painter, &mut taken, a, ra, None, &name, s);
        }
    }
    Dealt {
        showing: true,
        names: taken,
    }
}

/// A faint thread from each card to the next in time, from where its call is made
/// to where the next one is, so the deck's order reads as one line through it.
fn draw_thread(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    explorer: &Explorer,
    deck: &Deck,
    mode: TimeMode,
    order: &[(usize, f32)],
) {
    let mut by_time = order.to_vec();
    by_time.sort_by_key(|&(index, _)| index);
    for pair in by_time.windows(2) {
        let ((this, away), (next, next_away)) = (pair[0], pair[1]);
        if next != this + 1 {
            continue;
        }
        let (card, after) = (&deck.cards[this], &deck.cards[next]);
        // The next call is made inside this one, by its callee, or beside it.
        let from = if after.caller == card.callee {
            card.callee
        } else {
            card.caller
        };
        let point = |tile: usize, away: f32| {
            project(explorer.tile_point(tile)? + offset(mode, away, deck.step))
        };
        let (Some(a), Some(b)) = (point(from, away), point(after.caller, next_away)) else {
            continue;
        };
        let s = strength(away).min(strength(next_away));
        painter.line_segment(
            [a, b],
            egui::Stroke::new(1.5, egui::Color32::from_white_alpha(alpha(0.5 * s))),
        );
    }
}

/// A call's two ends on screen, each a node's middle and how far its edge is from it.
type Ends = ((egui::Pos2, f32), (egui::Pos2, f32));

/// Where `card`'s caller and callee land on screen, moved by `shift`.
fn ends(
    explorer: &Explorer,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    card: &Card,
    shift: Vec3,
) -> Option<Ends> {
    let end = |tile: usize| {
        let at = explorer.tile_point(tile)? + shift;
        let center = project(at)?;
        let top = at + Vec3::Y * node_size(explorer.graph(), tile) * 0.5;
        let radius = project(top).map_or(4.0, |p| p.distance(center)).max(3.5);
        Some((center, radius))
    };
    Some((end(card.caller)?, end(card.callee)?))
}

/// Opacity, as a byte, for a share of full.
fn alpha(share: f32) -> u8 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to a byte"
    )]
    let byte = (share.clamp(0.0, 1.0) * 255.0).round() as u8;
    byte
}

/// Which way time runs through the deck: `earlier` over the furthest card back in
/// time, `later` over the furthest ahead, each at its card's top corner.
fn draw_ends(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    deck: &Deck,
    mode: TimeMode,
    order: &[(usize, f32)],
) {
    let Some((low, high)) = deck.frame else {
        return;
    };
    let first = order.iter().map(|o| o.1).fold(f32::INFINITY, f32::min);
    let last = order.iter().map(|o| o.1).fold(f32::NEG_INFINITY, f32::max);
    let ends = [
        (
            first,
            Vec3::new(low.x, high.y, low.z),
            egui::Align2::LEFT_BOTTOM,
            "← earlier",
        ),
        (
            last,
            high.with_z(low.z),
            egui::Align2::RIGHT_BOTTOM,
            "later →",
        ),
    ];
    for (away, corner, align, text) in ends {
        if away.abs() < 0.5 || !away.is_finite() {
            continue;
        }
        if let Some(at) = project(corner + offset(mode, away, deck.step)) {
            painter.text(
                at + egui::vec2(0.0, -3.0),
                align,
                text,
                egui::FontId::proportional(11.0),
                ui::MUTED,
            );
        }
    }
}

/// A card's outline, and the map's module rings faintly on it, moved by `shift`.
fn draw_footprint(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    deck: &Deck,
    regions: &[(usize, Vec3, f32)],
    modules: &[Option<usize>],
    shift: Vec3,
    s: f32,
) {
    let Some((low, high)) = deck.frame else {
        return;
    };
    let corners = [
        low,
        Vec3::new(high.x, low.y, low.z),
        high.with_z(low.z),
        Vec3::new(low.x, high.y, low.z),
    ];
    let outline: Vec<egui::Pos2> = corners.iter().filter_map(|&c| project(c + shift)).collect();
    if outline.len() == 4 {
        painter.add(egui::Shape::convex_polygon(
            outline.clone(),
            egui::Color32::from_rgba_unmultiplied(150, 165, 200, alpha(0.08 * s)),
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(alpha(0.5 * s))),
        ));
    }
    // Only the rings of the modules the call runs between, so a card says where on
    // the map its call goes without repeating all of it.
    for &(cluster, middle, radius) in regions.iter().filter(|r| modules.contains(&Some(r.0))) {
        let color = ui::egui_color(lane_color(cluster % PALETTE.len()));
        let ring: Vec<egui::Pos2> = (0..=40)
            .filter_map(|i| {
                #[expect(clippy::cast_precision_loss, reason = "a small count")]
                let turn = i as f32 / 40.0 * std::f32::consts::TAU;
                project(middle + Vec3::new(turn.cos(), turn.sin(), 0.0) * radius + shift)
            })
            .collect();
        if ring.len() > 1 {
            painter.add(egui::Shape::line(
                ring,
                egui::Stroke::new(1.0, color.gamma_multiply(0.4 * s)),
            ));
        }
    }
}

/// A call from `a` to `b`, clear of their discs, with its head at `b`.
fn arrow(painter: &egui::Painter, ((a, ra), (b, rb)): Ends, color: egui::Color32, width: f32) {
    let along = b - a;
    let length = along.length();
    if length <= ra + rb + 4.0 {
        return;
    }
    let dir = along / length;
    let (start, tip) = (a + dir * (ra + 1.0), b - dir * (rb + 1.0));
    let head = 5.0 + width * 1.5;
    let back = tip - dir * head;
    let side = egui::vec2(-dir.y, dir.x) * head * 0.5;
    // On a dark casing, so it reads over the map's own links.
    let casing = egui::Color32::from_black_alpha(color.a().saturating_add(40).min(200));
    painter.line_segment([start, back], egui::Stroke::new(width + 3.0, casing));
    painter.add(egui::Shape::convex_polygon(
        vec![tip, back + side, back - side],
        color,
        egui::Stroke::new(1.5, casing),
    ));
    painter.line_segment([start, back], egui::Stroke::new(width, color));
}

/// A node on a card: a disc in its color, lit from the top left, so it reads as a
/// marker on the card rather than a hole in it.
fn disc(painter: &egui::Painter, center: egui::Pos2, radius: f32, fill: egui::Color32, s: f32) {
    let shade = |share: f32, bright: f32| {
        let [r, g, b, _] = fill.to_array();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to a byte"
        )]
        let tone = |c: u8| (f32::from(c) * bright).min(255.0) as u8;
        egui::Color32::from_rgba_unmultiplied(tone(r), tone(g), tone(b), alpha(share))
    };
    painter.circle_filled(
        center,
        radius + 1.5,
        egui::Color32::from_black_alpha(alpha(0.7 * s.max(0.3))),
    );
    painter.circle_filled(center, radius, shade(0.95 * s.max(0.3), 0.45));
    let light = center + egui::vec2(-0.22, -0.22) * radius;
    painter.circle_filled(light, radius * 0.62, shade(0.9 * s.max(0.3), 0.95));
    painter.circle_filled(
        light + egui::vec2(-0.15, -0.15) * radius,
        radius * 0.22,
        egui::Color32::from_white_alpha(alpha(0.5 * s)),
    );
}

/// A name beside a node on a card, on a dark pill, led by a badge filled like its node;
/// left out where it would cover a name already placed.
fn label(
    painter: &egui::Painter,
    taken: &mut Vec<egui::Rect>,
    node: egui::Pos2,
    radius: f32,
    badge: Option<(String, egui::Color32)>,
    name: &str,
    s: f32,
) {
    let text = egui::Color32::from_rgb(225, 229, 238);
    let lead = badge.map(|(number, fill)| (format!(" {number} "), ui::ON_FILL, Some(fill)));
    let job = ui::label_job(lead, name, None, text, 12.0, false);
    let galley = painter.layout_job(job);
    let extent = galley.size();
    let gap = radius + 5.0;
    let spots = [
        egui::Align2::LEFT_CENTER.anchor_size(node + egui::vec2(gap, 0.0), extent),
        egui::Align2::RIGHT_CENTER.anchor_size(node - egui::vec2(gap, 0.0), extent),
        egui::Align2::CENTER_BOTTOM.anchor_size(node - egui::vec2(0.0, gap), extent),
        egui::Align2::CENTER_TOP.anchor_size(node + egui::vec2(0.0, gap), extent),
    ];
    let pad = egui::vec2(5.0, 2.0);
    let Some(rect) = spots
        .into_iter()
        .find(|r| !taken.iter().any(|t| t.intersects(r.expand2(pad))))
    else {
        return;
    };
    let pill = rect.expand2(pad);
    taken.push(pill);
    let mut painter = painter.clone();
    painter.multiply_opacity(s.max(0.7));
    painter.rect_filled(pill, 4.0, ui::PILL);
    painter.galley(rect.min, galley, egui::Color32::WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_run_depth_first() {
        // 0 calls 1 then 4; 1 calls 2 then 3.
        let tree = |n: u32| match n {
            0 => vec![1, 4],
            1 => vec![2, 3],
            _ => vec![],
        };
        assert_eq!(
            calls_in_order(0, tree),
            vec![(0, 1), (1, 2), (1, 3), (0, 4)]
        );
    }

    #[test]
    fn the_cursor_sits_at_its_call_or_the_nearest_above() {
        let calls = [1, 2, 3, 4];
        let parent = |n: u32| match n {
            1 | 4 => Some(0),
            2 | 3 => Some(1),
            9 => Some(3),
            _ => None,
        };
        assert!((cursor_place(&calls, Some(3), parent) - 2.0).abs() < 1e-6);
        assert!(
            (cursor_place(&calls, Some(9), parent) - 2.0).abs() < 1e-6,
            "a call deeper than the deck sits at its caller's card"
        );
        assert!(
            (cursor_place(&calls, Some(0), parent) + 1.0).abs() < 1e-6,
            "the journey's start is before the first call"
        );
        assert!((cursor_place(&calls, None, parent) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn cards_fade_with_time_from_the_cursor() {
        assert!((strength(0.0) - 1.0).abs() < 1e-6);
        assert!(
            (strength(1.0) - NEXT).abs() < 1e-6,
            "continuous at the next card"
        );
        assert!(strength(-3.0) < strength(-2.0), "behind as well as ahead");
        assert!((strength(2.0) - strength(-2.0)).abs() < 1e-6);
        assert!(strength(7.0) > 0.0, "never quite gone");
    }

    #[test]
    fn a_far_call_does_not_stretch_the_cards() {
        let mut points: Vec<Vec3> = (0..20)
            .map(|i| Vec3::new(i as f32, (i % 5) as f32, 0.0))
            .collect();
        points.push(Vec3::new(500.0, 0.0, 0.0));
        let (low, high) = footprint(&points).unwrap();
        assert!(high.x < 40.0, "the outlier is left out: {high}");
        assert!(low.x < 2.0 && high.x > 17.0, "most of the rest is in");
        assert!(footprint(&[]).is_none());
    }

    #[test]
    fn the_cursors_card_lies_on_the_map() {
        let step = Vec3::new(2.0, 1.0, 3.0);
        for mode in [TimeMode::Fan, TimeMode::Stack] {
            assert_eq!(offset(mode, 0.0, step), Vec3::ZERO);
        }
        let (before, after) = (
            offset(TimeMode::Fan, -2.0, step),
            offset(TimeMode::Fan, 2.0, step),
        );
        assert!(
            after.x > 0.0 && before.x < 0.0,
            "fanned left to right in time"
        );
        assert!(after.y > 0.0, "and up, clear of the journey docked below");
        assert!(
            (before.z - after.z).abs() < 1e-6 && after.z < 0.0,
            "both sides further back than the cursor's card"
        );
        let stacked = offset(TimeMode::Stack, 2.0, step);
        assert_eq!(stacked.truncate(), Vec2::ZERO, "stacked straight back");
        assert!(stacked.z < 0.0, "what comes next further back");
    }
}
