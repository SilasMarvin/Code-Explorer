//! Strata: the graph as a stack of glass layers, one per step down the flow of calls.
//! Entry points and whatever nothing calls sit on the top layer, and every call steps
//! down at least one layer, so what calls the focus is always above it and what it
//! calls always below. Each module stands on its place on the map as a translucent
//! column through the layers its functions occupy: a function keeps its map position
//! and sits inside its module's column, on its layer. Calls inside a cycle cannot step
//! down; they bow up over their layer instead, which is what makes them stand out.

use std::{collections::HashSet, f32::consts::FRAC_PI_2};

use bevy::prelude::*;

use crate::{
    camera::{ANGLED, OrbitCamera},
    explorer::Explorer,
    graph_view::{self, Relation, relation},
    scene::{PALETTE, Tile, lane_color},
};

/// The least height from one layer to the next.
const LAYER: f32 = 9.0;
/// How the camera looks at the layers: turned a little, and down on them.
pub(crate) const POSE: (f32, f32) = (-0.3, -0.3);
/// Layers either side of the focus a full step apart.
const NEAR: f32 = 3.0;
/// The share of a step the layers further out take.
const SQUEEZE: f32 = 0.35;
/// Room round the map on every layer.
const MARGIN: f32 = 6.0;

/// Which layer each function sits on.
#[derive(Debug, Clone, Default)]
pub(crate) struct Strata {
    ranks: Vec<u32>,
    depth: u32,
    /// Height from one layer to the next, near the focus.
    spacing: f32,
    /// The focus's layer: layers further than [`NEAR`] from it are squeezed.
    focus: u32,
}

impl Strata {
    /// Layers `count` functions by their `(caller, callee)` calls; `entries` stay on
    /// top even when they call nothing deep. The stack stands about as tall as the
    /// map, `span` across, is wide, so steps down read beside steps across.
    pub(crate) fn new(
        count: usize,
        calls: &[(usize, usize)],
        entries: &HashSet<usize>,
        span: f32,
    ) -> Self {
        let ranks = ranks(count, calls, entries);
        let depth = ranks.iter().copied().max().unwrap_or(0);
        #[expect(clippy::cast_precision_loss, reason = "a layer count")]
        let spacing = (span / (depth + 1) as f32).clamp(LAYER, LAYER * 3.0);
        Self {
            ranks,
            depth,
            spacing,
            focus: 0,
        }
    }

    /// Where `tile`, at map point `at`, sits: the map lies flat, its top away from
    /// the camera, on the tile's layer.
    pub(crate) fn place(&self, tile: usize, at: [f32; 3]) -> Vec3 {
        Vec3::new(at[0], -self.height(self.rank(tile)), -at[1])
    }

    /// Centers the layers on `rank`, the focus's. Returns whether that moved them.
    pub(crate) fn focus_on(&mut self, rank: u32) -> bool {
        let moved = self.focus != rank;
        self.focus = rank;
        moved
    }

    /// How far below the top layer `rank` is: a full step apart near the focus, and
    /// squeezed together beyond, so a deep stack still fits on screen around it.
    #[expect(clippy::cast_precision_loss, reason = "layer counts")]
    fn height(&self, rank: u32) -> f32 {
        let from = rank as f32 - self.focus as f32;
        let near = from.clamp(-NEAR, NEAR);
        (self.focus as f32 + near + (from - near) * SQUEEZE) * self.spacing
    }

    /// The layer `tile` sits on, 0 being the top.
    pub(crate) fn rank(&self, tile: usize) -> u32 {
        self.ranks.get(tile).copied().unwrap_or(0)
    }

    /// The bottom layer.
    pub(crate) fn depth(&self) -> u32 {
        self.depth
    }
}

/// Each function's layer: one below the deepest of its callers, so every call steps
/// down, counting a cycle as one function. What nothing calls starts on top, unless
/// it is no entry point and calls something, when it hangs just above the highest of
/// its calls; what neither calls nor is called rests on the bottom, with the leaves.
pub(crate) fn ranks(count: usize, calls: &[(usize, usize)], entries: &HashSet<usize>) -> Vec<u32> {
    let (component, order) = components(count, calls);
    let groups = order.len();
    let mut rank = vec![0_u32; groups];
    let mut called = vec![false; groups];
    let mut calling = vec![false; groups];
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); groups];
    for &(from, to) in calls {
        let (a, b) = (component[from], component[to]);
        if a != b {
            out[a].push(b);
            called[b] = true;
            calling[a] = true;
        }
    }
    // Components come in the order calls run, so a caller's layer is final first.
    for group in 0..groups {
        for &next in &out[group] {
            rank[next] = rank[next].max(rank[group] + 1);
        }
    }
    let entry_groups: HashSet<usize> = entries.iter().map(|&t| component[t]).collect();
    for group in 0..groups {
        if called[group] || !calling[group] || entry_groups.contains(&group) {
            continue;
        }
        if let Some(highest) = out[group].iter().map(|&next| rank[next]).min() {
            rank[group] = highest - 1;
        }
    }
    let bottom = rank.iter().copied().max().unwrap_or(0);
    for group in 0..groups {
        if !called[group] && !calling[group] && !entry_groups.contains(&group) {
            rank[group] = bottom;
        }
    }
    component.iter().map(|&group| rank[group]).collect()
}

/// Groups functions that call each other round in a cycle, numbering the groups in
/// the order calls run between them: a group's callers come before it. Returns each
/// function's group and the groups in that order (Kosaraju's, without recursion).
fn components(count: usize, calls: &[(usize, usize)]) -> (Vec<usize>, Vec<usize>) {
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut into: Vec<Vec<usize>> = vec![Vec::new(); count];
    for &(from, to) in calls {
        if from < count && to < count {
            out[from].push(to);
            into[to].push(from);
        }
    }
    // Every function, by when a walk along calls finishes with it.
    let mut finished = Vec::with_capacity(count);
    let mut seen = vec![false; count];
    for start in 0..count {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![(start, 0)];
        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            if let Some(&to) = out[node].get(*next) {
                *next += 1;
                if !seen[to] {
                    seen[to] = true;
                    stack.push((to, 0));
                }
            } else {
                finished.push(node);
                stack.pop();
            }
        }
    }
    // Walking back against calls, latest finished first, gathers one cycle at a time,
    // callers' before callees'.
    let mut component = vec![usize::MAX; count];
    let mut order = Vec::new();
    for &start in finished.iter().rev() {
        if component[start] != usize::MAX {
            continue;
        }
        let group = order.len();
        order.push(group);
        component[start] = group;
        let mut stack = vec![start];
        while let Some(node) = stack.pop() {
            for &from in &into[node] {
                if component[from] == usize::MAX {
                    component[from] = group;
                    stack.push(from);
                }
            }
        }
    }
    (component, order)
}

/// A module standing through the layers: where it is on the map, how wide, and the
/// top and bottom layers its functions sit on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Column {
    pub(crate) cluster: usize,
    pub(crate) middle: Vec2,
    pub(crate) radius: f32,
    pub(crate) top: u32,
    pub(crate) bottom: u32,
}

impl Column {
    /// The middle of the column on layer `rank`, or on the nearest layer it reaches.
    pub(crate) fn at(&self, strata: &Strata, rank: u32) -> Vec3 {
        let rank = rank.clamp(self.top, self.bottom);
        Vec3::new(self.middle.x, -strata.height(rank), -self.middle.y)
    }
}

/// Every module's column, from where its functions are on the map.
pub(crate) fn columns(explorer: &Explorer, strata: &Strata) -> Vec<Column> {
    let graph = explorer.graph();
    let mut members: Vec<Vec<(Vec2, u32)>> = vec![Vec::new(); graph.cluster_count()];
    for (tile, at) in graph.positions.iter().enumerate() {
        if let Some(list) = graph.cluster(tile).and_then(|c| members.get_mut(c)) {
            list.push((Vec2::new(at[0], at[1]), strata.rank(tile)));
        }
    }
    members
        .iter()
        .enumerate()
        .filter(|(_, points)| !points.is_empty())
        .map(|(cluster, points)| {
            #[expect(clippy::cast_precision_loss, reason = "a member count")]
            let middle = points.iter().map(|p| p.0).sum::<Vec2>() / points.len() as f32;
            let mut distances: Vec<f32> = points.iter().map(|p| p.0.distance(middle)).collect();
            distances.sort_by(f32::total_cmp);
            let radius = distances[(distances.len() * 9 / 10).min(distances.len() - 1)] + 3.0;
            let ranks = points.iter().map(|p| p.1);
            Column {
                cluster,
                middle,
                radius,
                top: ranks.clone().min().unwrap_or(0),
                bottom: ranks.max().unwrap_or(0),
            }
        })
        .collect()
}

/// Each module's region for naming it: on the focus's layer, or as near to it as the
/// column reaches, so a module is named where the eye already is.
pub(crate) fn regions(explorer: &Explorer, strata: &Strata) -> Vec<(usize, Vec3, f32)> {
    let level = explorer.graph_focus().map_or(0, |t| strata.rank(t));
    columns(explorer, strata)
        .into_iter()
        .map(|column| (column.cluster, column.at(strata, level), column.radius))
        .collect()
}

/// The map's extent, on every layer: its lower and upper corners.
fn extent(explorer: &Explorer) -> (Vec2, Vec2) {
    let points = explorer
        .graph()
        .positions
        .iter()
        .map(|p| Vec2::new(p[0], p[1]));
    let (low, high) = points.fold(
        (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
        |(low, high), p| (low.min(p), high.max(p)),
    );
    if low.x > high.x {
        return (Vec2::ZERO, Vec2::ZERO);
    }
    (low - MARGIN, high + MARGIN)
}

/// Where each layer is named: on a plumb line through the focus, so how far above
/// or below it something sits is read against it.
pub(crate) fn layer_marks(explorer: &Explorer, strata: &Strata) -> Vec<(Vec3, u32)> {
    let Some(focus) = explorer.graph_focus().and_then(|t| explorer.tile_point(t)) else {
        return Vec::new();
    };
    (0..=strata.depth())
        .map(|rank| (focus.with_y(-strata.height(rank)), rank))
        .collect()
}

/// The neighbor of the focus a node two calls away goes through: the caller of the
/// focus it calls, upstream, or the callee of the focus that calls it, downstream.
pub(crate) fn through(explorer: &Explorer, tile: usize, upstream: bool) -> Option<usize> {
    let reach = explorer.reach()?;
    let graph = explorer.graph();
    if upstream {
        graph
            .callees(tile)
            .find(|t| reach.upstream.get(t) == Some(&1))
    } else {
        graph
            .callers(tile)
            .find(|t| reach.downstream.get(t) == Some(&1))
    }
}

/// Whether a call cannot step down its layers, as inside a cycle.
pub(crate) fn climbs(strata: &Strata, from: usize, to: usize) -> bool {
    strata.rank(to) <= strata.rank(from)
}

/// A call that cannot step down, as a curve bowing up over its layer from `a` to `b`,
/// ending in an arrowhead.
pub(crate) fn draw_bow(gizmos: &mut Gizmos, a: Vec3, b: Vec3, color: Color) {
    let lift = Vec3::Y * (a.distance(b) * 0.4 + LAYER * 0.6);
    let control = (a + b) / 2.0 + lift;
    let points: Vec<Vec3> = (0..=16)
        .map(|step| {
            #[expect(clippy::cast_precision_loss, reason = "a small count")]
            let t = step as f32 / 16.0;
            a.lerp(control, t).lerp(control.lerp(b, t), t)
        })
        .collect();
    gizmos.linestrip(points.iter().copied(), color);
    if let [.., before, last] = points.as_slice() {
        gizmos.arrow(*before, *last, color).with_tip_length(1.1);
    }
}

/// The focus and what is two calls either way of it.
fn lit(explorer: &Explorer) -> Vec<usize> {
    let reach = explorer.reach();
    (0..explorer.map().tiles.len())
        .filter(|&tile| {
            explorer.graph_focus() == Some(tile)
                || reach.and_then(|r| relation(r, tile)).is_some_and(|r| {
                    matches!(r, Relation::Upstream(h) | Relation::Downstream(h)
                        if h <= graph_view::STRATA_REACH)
                })
        })
        .collect()
}

/// The modules the focus and what is two calls either way of it live in.
pub(crate) fn lit_clusters(explorer: &Explorer) -> HashSet<usize> {
    lit(explorer)
        .into_iter()
        .filter_map(|tile| explorer.graph().cluster(tile))
        .collect()
}

/// Lying flat on a layer.
fn flat(at: Vec3) -> Isometry3d {
    Isometry3d::new(at, Quat::from_rotation_x(FRAC_PI_2))
}

/// The layers' edges, the focus's brightest, and a ring round each module wherever
/// one of its functions near the focus sits, so that function reads as inside it.
pub(crate) fn draw(explorer: &Explorer, strata: &Strata, gizmos: &mut Gizmos) {
    let (low, high) = extent(explorer);
    let size = high - low;
    let middle = (low + high) / 2.0;
    let focus_rank = explorer.graph_focus().map(|t| strata.rank(t));
    for rank in 0..=strata.depth() {
        let at = Vec3::new(middle.x, -strata.height(rank), -middle.y);
        // The more layers, the fainter each, so a deep stack does not fog over.
        #[expect(clippy::cast_precision_loss, reason = "a layer count")]
        let faint = (0.6 / (strata.depth() + 1) as f32).clamp(0.04, 0.12);
        let alpha = if Some(rank) == focus_rank { 0.5 } else { faint };
        gizmos.rect(flat(at), size, Color::srgba(0.7, 0.78, 0.95, alpha));
    }
    // The plumb line, with a tick on each layer.
    let marks = layer_marks(explorer, strata);
    if let (Some(&(top, _)), Some(&(bottom, _))) = (marks.first(), marks.last()) {
        let line = Color::srgba(0.85, 0.9, 1.0, 0.35);
        gizmos.line(top, bottom, line);
        for &(at, _) in &marks {
            gizmos.line(at - Vec3::X * 1.5, at + Vec3::X * 1.5, line);
        }
    }
    let lit = lit(explorer);
    // Each lit node sits on a ring on its layer, in its module's color.
    for &tile in &lit {
        let (Some(at), Some(cluster)) = (explorer.tile_point(tile), explorer.graph().cluster(tile))
        else {
            continue;
        };
        let radius = graph_view::node_size(explorer.graph(), tile) * 0.5 + 1.0;
        gizmos
            .circle(
                flat(at),
                radius,
                lane_color(cluster % PALETTE.len()).with_alpha(0.7),
            )
            .resolution(24);
    }
    let near: HashSet<(usize, u32)> = lit
        .iter()
        .filter_map(|&tile| Some((explorer.graph().cluster(tile)?, strata.rank(tile))))
        .collect();
    let level = explorer.graph_focus().map_or(0, |t| strata.rank(t));
    for column in columns(explorer, strata) {
        let color = lane_color(column.cluster % PALETTE.len());
        // A module away from the focus leaves only its footprint, on the focus's
        // layer, so the map still shows.
        if !near.iter().any(|&(cluster, _)| cluster == column.cluster) {
            gizmos
                .circle(
                    flat(column.at(strata, level)),
                    column.radius,
                    color.with_alpha(0.06),
                )
                .resolution(48);
            continue;
        }
        for rank in [column.top, column.bottom] {
            gizmos
                .circle(
                    flat(column.at(strata, rank)),
                    column.radius,
                    color.with_alpha(0.16),
                )
                .resolution(48);
        }
        for rank in column.top..=column.bottom {
            if near.contains(&(column.cluster, rank)) {
                gizmos
                    .circle(
                        flat(column.at(strata, rank)),
                        column.radius,
                        color.with_alpha(0.6),
                    )
                    .resolution(64);
            }
        }
    }
}

/// The glass: a pane on the focus's layer and a tube per module near it.
#[derive(Component, Debug)]
pub(crate) struct Glass;

/// What the glass is made of.
#[derive(Resource, Debug)]
pub(crate) struct GlassLook {
    pane: Handle<Mesh>,
    tube: Handle<Mesh>,
    focus_layer: Handle<StandardMaterial>,
    columns: Vec<Handle<StandardMaterial>>,
}

pub(crate) fn create_glass(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut glass = |color: Color| {
        materials.add(StandardMaterial {
            base_color: color,
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            ..default()
        })
    };
    let focus_layer = glass(Color::srgba(0.75, 0.82, 1.0, 0.02));
    let columns = (0..PALETTE.len())
        .map(|color| glass(lane_color(color).with_alpha(0.028)))
        .collect();
    commands.insert_resource(GlassLook {
        pane: meshes.add(Plane3d::default().mesh().size(1.0, 1.0)),
        tube: meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(48)),
        focus_layer,
        columns,
    });
}

/// What the glass was built for: how often nodes had moved, the focus, its layer and
/// whether the strata were on.
type GlassKey = (u64, Option<usize>, Option<u32>, bool);

/// Builds the glass again whenever the nodes move, the focus changes layer or the
/// strata come on or off.
pub(crate) fn build_glass(
    mut commands: Commands,
    explorer: Res<Explorer>,
    look: Res<GlassLook>,
    panes: Query<Entity, With<Glass>>,
    mut built: Local<Option<GlassKey>>,
) {
    let strata = explorer.strata();
    let key = (
        explorer.moved(),
        explorer.graph_focus(),
        strata.and_then(|s| Some(s.rank(explorer.graph_focus()?))),
        strata.is_some(),
    );
    if *built == Some(key) {
        return;
    }
    *built = Some(key);
    for entity in &panes {
        commands.entity(entity).despawn();
    }
    let Some(strata) = strata else {
        return;
    };
    let (low, high) = extent(&explorer);
    let middle = (low + high) / 2.0;
    let size = high - low;
    // Only the focus's layer is glazed; the rest are drawn as edges, so a deep stack
    // stays clear.
    if let Some(rank) = key.2 {
        commands.spawn((
            Glass,
            Pickable::IGNORE,
            Mesh3d(look.pane.clone()),
            MeshMaterial3d(look.focus_layer.clone()),
            Transform::from_xyz(middle.x, -strata.height(rank), -middle.y)
                .with_scale(Vec3::new(size.x, 1.0, size.y)),
        ));
    }
    let lit = lit_clusters(&explorer);
    for column in columns(&explorer, strata) {
        if !lit.contains(&column.cluster) {
            continue;
        }
        let top = strata.height(column.top) - strata.spacing * 0.4;
        let bottom = strata.height(column.bottom) + strata.spacing * 0.4;
        commands.spawn((
            Glass,
            Pickable::IGNORE,
            Mesh3d(look.tube.clone()),
            MeshMaterial3d(look.columns[column.cluster % PALETTE.len()].clone()),
            Transform::from_xyz(column.middle.x, -(top + bottom) / 2.0, -column.middle.y)
                .with_scale(Vec3::new(column.radius, bottom - top, column.radius)),
        ));
    }
}

/// Z lays the graph out in strata, or back as it was.
pub(crate) fn toggle(keys: &ButtonInput<KeyCode>, explorer: &mut Explorer) {
    if keys.just_pressed(KeyCode::KeyZ) {
        explorer.set_strata(explorer.strata().is_none());
    }
}

/// Moves every node to its place when the strata come on or off, or center on
/// another layer.
pub(crate) fn place_nodes(
    explorer: Res<Explorer>,
    mut last: Local<Option<(bool, u64)>>,
    mut nodes: Query<(&Tile, &mut Transform)>,
) {
    let now = (explorer.strata().is_some(), explorer.moved());
    if *last == Some(now) {
        return;
    }
    *last = Some(now);
    for (tile, mut transform) in &mut nodes {
        if let Some(at) = explorer.tile_point(tile.0) {
            transform.translation = at;
        }
    }
}

/// Turns to look down on the layers when they come on, and back when they go.
pub(crate) fn frame(
    explorer: Res<Explorer>,
    mut last: Local<Option<bool>>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    let on = explorer.strata().is_some();
    if *last == Some(on) {
        return;
    }
    let opening = last.is_none();
    *last = Some(on);
    if opening && !on {
        return;
    }
    for mut camera in &mut cameras {
        camera.turn_to(if on { POSE } else { ANGLED });
        graph_view::orbit_anchor(&explorer, &mut camera);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_call_steps_down() {
        // 0 → 1 → 2 and 0 → 2: 2 sits below 1, not beside it.
        let calls = [(0, 1), (1, 2), (0, 2)];
        assert_eq!(ranks(3, &calls, &HashSet::new()), [0, 1, 2]);
    }

    #[test]
    fn a_cycle_shares_a_layer() {
        // 0 → 1 ⇄ 2 → 3.
        let calls = [(0, 1), (1, 2), (2, 1), (2, 3)];
        assert_eq!(ranks(4, &calls, &HashSet::new()), [0, 1, 1, 2]);
    }

    #[test]
    fn what_nothing_calls_hangs_above_its_calls_unless_an_entry() {
        // 0 → 1 → 2 → 3; 4 → 3 hangs just above 3; 5 → 3 is an entry and stays on
        // top; 6 is alone, with the leaves.
        let calls = [(0, 1), (1, 2), (2, 3), (4, 3), (5, 3)];
        let entries = HashSet::from([0, 5]);
        assert_eq!(ranks(7, &calls, &entries), [0, 1, 2, 3, 2, 0, 3]);
    }

    #[test]
    fn layers_far_from_the_focus_squeeze_together() {
        let calls: Vec<(usize, usize)> = (0..10).map(|t| (t, t + 1)).collect();
        let mut strata = Strata::new(11, &calls, &HashSet::new(), 0.0);
        strata.focus_on(5);
        let y = |tile| strata.place(tile, [0.0; 3]).y;
        assert!(
            (y(5) - y(6) - LAYER).abs() < 1e-4,
            "next to it, a full step"
        );
        assert!(
            (y(8) - y(9) - LAYER * SQUEEZE).abs() < 1e-4,
            "far below, squeezed"
        );
        assert!(
            (y(0) - y(1) - LAYER * SQUEEZE).abs() < 1e-4,
            "far above, squeezed"
        );
    }

    #[test]
    fn layers_stack_down_from_the_map() {
        let strata = Strata::new(3, &[(0, 1), (1, 2)], &HashSet::new(), 0.0);
        let at = strata.place(2, [3.0, 4.0, 1.0]);
        assert_eq!(at, Vec3::new(3.0, -2.0 * LAYER, -4.0));
    }
}
