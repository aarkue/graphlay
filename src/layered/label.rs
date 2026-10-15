//! Edge labels: room reserved in the layer gaps, and where each label is drawn.
//!
//! [`reserve_label_gaps`] reserves room only in the gap a label sits in. [`place_edge_labels`] runs
//! after routing, because a label has to miss node boxes, other labels and polylines. A label
//! always gets a position, since the renderer draws it anyway.

use super::state::LayeredState;
use crate::geometry::fp;
use crate::geometry::hit::point_seg_dist_coarse;
use crate::geometry::{Pt, EPS};
use crate::options::{LabelPlacement, LabelReservation, LayoutOptions};
use crate::sort::SortDyn;

/// Floor on [`crate::options::LabelOptions::clearance`] when it divides a shortfall, so a zero
/// clearance does not divide by zero.
const MIN_CLEARANCE: f64 = 1e-6;

/// Bounds (px) on the cell side of the obstacle grid.
const GRID_CELL: (f64, f64) = (48.0, 1024.0);

/// The layer gap a chain's label belongs to, the middle one it crosses.
fn gap_of(st: &LayeredState, chain: &[usize]) -> Option<usize> {
    let (first, last) = (*chain.first()?, *chain.last()?);
    let (rf, rl) = (
        st.nodes[first].layer_clamped(),
        st.nodes[last].layer_clamped(),
    );
    let (r0, r1) = (rf.min(rl), rf.max(rl));
    (r1 > r0).then(|| r0 + (r1 - r0 - 1) / 2)
}

/// Layer-axis extent of `arc`'s label, or `None` when it has none.
fn label_extents(st: &LayeredState, sizes: &[(f64, f64)], arc: usize) -> Option<(f64, f64)> {
    match sizes.get(arc) {
        Some(&(w, h)) if w > 0.0 && h > 0.0 => Some(st.axis.extents(w, h)),
        _ => None,
    }
}

/// Space to keep after each layer, the base layer gap widened to the widest label that needs it.
///
/// Under [`LabelReservation::WhenNeeded`], a label whose cross extent fits in `node_gap` with
/// clearance needs no room, because [`place_edge_labels`] can put it between two rows.
pub fn reserve_label_gaps(st: &LayeredState, sizes: &[(f64, f64)], o: &LayoutOptions) -> Vec<f64> {
    let mut gap = vec![o.spacing.layer_gap; st.layers.len()];
    if sizes.is_empty() || o.labels.reservation == LabelReservation::Never {
        return gap;
    }
    for (&arc, chain) in &st.chains {
        let (Some((lw, cw)), Some(g)) = (label_extents(st, sizes, arc), gap_of(st, chain)) else {
            continue;
        };
        if o.labels.reservation == LabelReservation::WhenNeeded
            && cw + 2.0 * o.labels.clearance <= o.spacing.node_gap
        {
            continue;
        }
        if let Some(slot) = gap.get_mut(g) {
            *slot = slot.max(lw + o.spacing.edge_label_margin);
        }
    }
    gap
}

/// Centre of each edge's label box in flow space, or `None` for an unlabelled edge. Call after
/// routing. [`LabelPlacement::Candidates`] searches the polyline for the fewest collisions.
pub fn place_edge_labels(
    st: &LayeredState,
    sizes: &[(f64, f64)],
    gap_after: &[f64],
    o: &LayoutOptions,
) -> Vec<Option<(f64, f64)>> {
    let reserved = gap_centre_anchors(st, sizes, gap_after, o);
    if o.labels.placement == LabelPlacement::GapCentre {
        return reserved;
    }
    let nodes: Vec<(Pt, Pt)> = st
        .nodes
        .iter()
        .take(st.n_real)
        .filter(|nd| nd.w_along > 0.0 && nd.w_cross > 0.0)
        .map(|nd| ((nd.along, nd.cross), (nd.w_along / 2.0, nd.w_cross / 2.0)))
        .collect();
    let halves: Vec<Option<Pt>> = (0..st.routes.len())
        .map(|arc| label_extents(st, sizes, arc).map(|(a, c)| (a / 2.0, c / 2.0)))
        .collect();
    let scene = LabelScene {
        nodes: &nodes,
        routes: &st.routes,
        // Self-loops are not in `routes`, but labels must still avoid them.
        loops: st.loop_routes.iter().map(|(&e, r)| (e, &r[..])).collect(),
        thickness: &st.thickness,
        labels: &halves,
        reserved: &reserved,
    };
    place_labels(&scene, o)
}

/// The centre of the gap each label was reserved in. Labels sharing a gap are stacked along the
/// cross axis by greedy interval packing, so the gap only needs the widest label's width.
fn gap_centre_anchors(
    st: &LayeredState,
    sizes: &[(f64, f64)],
    gap_after: &[f64],
    o: &LayoutOptions,
) -> Vec<Option<(f64, f64)>> {
    let mut out = vec![None; st.routes.len()];
    if sizes.is_empty() {
        return out;
    }
    let gap_centre: Vec<f64> = st
        .layers
        .iter()
        .enumerate()
        .map(|(li, layer)| {
            let along = layer.first().map_or(0.0, |&n| st.nodes[n].along);
            let half = layer
                .iter()
                .map(|&n| st.nodes[n].w_along / 2.0)
                .fold(0.0, f64::max);
            along + half + gap_after.get(li).copied().unwrap_or(o.spacing.layer_gap) / 2.0
        })
        .collect();

    // Per gap: (cross position where the edge crosses it, arc, label cross extent).
    let mut per_gap: Vec<Vec<(f64, usize, f64)>> = vec![vec![]; st.layers.len()];
    for (&arc, chain) in &st.chains {
        let (Some((_, ow)), Some(g)) = (label_extents(st, sizes, arc), gap_of(st, chain)) else {
            continue;
        };
        if g >= per_gap.len() {
            continue;
        }
        // The chain ascends one rank per step, so gap `g` lies between offsets `g - base` and
        // `g - base + 1`.
        let base = st.nodes[chain[0]].layer_clamped();
        let (Some(&a), Some(&b)) = (chain.get(g - base), chain.get(g - base + 1)) else {
            continue;
        };
        per_gap[g].push(((st.nodes[a].cross + st.nodes[b].cross) / 2.0, arc, ow));
    }

    for (g, mut labels) in per_gap.into_iter().enumerate() {
        if labels.is_empty() {
            continue;
        }
        labels.sort_unstable_by_dyn(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut cursor = f64::NEG_INFINITY;
        for (cross, arc, ow) in labels {
            let top = (cross - ow / 2.0).max(cursor);
            cursor = top + ow + o.spacing.edge_label_stack_gap;
            if let Some(slot) = out.get_mut(arc) {
                *slot = Some((gap_centre[g], top + ow / 2.0));
            }
        }
    }
    out
}

// The placement search sees the drawing only through `LabelScene`, so it can also run in output
// space. Its charges match what `metrics::labels` counts.

/// What label placement needs to know about the drawing. All per-edge slices share one edge index
/// and one coordinate space.
pub(crate) struct LabelScene<'a> {
    /// Node boxes as `(centre, half-extents)`.
    pub nodes: &'a [(Pt, Pt)],
    /// The polyline of each edge, empty if it could not be routed.
    pub routes: &'a [Vec<Pt>],
    /// Self-loop polylines, tagged with their edge index.
    pub loops: Vec<(usize, &'a [Pt])>,
    /// Drawn stroke width per edge, so a label beside its line clears the stroke.
    pub thickness: &'a [f64],
    /// Half-extents of each edge's label box, `None` for no label.
    pub labels: &'a [Option<Pt>],
    /// The reserved position of each label, if any. It is offered as the last candidate.
    pub reserved: &'a [Option<Pt>],
}

impl LabelScene<'_> {
    /// The polyline of `edge`, or an empty one.
    fn route(&self, edge: usize) -> &[Pt] {
        self.routes.get(edge).map_or(&[][..], |r| &r[..])
    }
}

/// Cost of overlapping a node box, a placed label or a foreign polyline, ordered by how much a
/// reader loses. These dwarf the clearance terms so no overlap is traded for daylight.
const NODE_HIT: f64 = 1000.0;
const LABEL_HIT: f64 = 900.0;
const EDGE_HIT: f64 = 400.0;
/// Cost of clearing an obstacle by less than `clearance`, pro rata.
const NODE_NEAR: f64 = 40.0;
const LABEL_NEAR: f64 = 30.0;
const EDGE_NEAR: f64 = 12.0;
/// Cost per px between the label's centre and its own polyline.
const DETACH: f64 = 0.8;
/// Cost per unit of `|fraction - 0.5|`, preferring mid-edge labels.
const OFF_CENTRE: f64 = 30.0;

/// An axis-aligned box in flow space: centre and half-extents.
#[derive(Clone, Copy)]
struct Box2 {
    along: f64,
    cross: f64,
    ha: f64,
    hc: f64,
}

impl Box2 {
    fn new(centre: Pt, ha: f64, hc: f64) -> Self {
        Box2 {
            along: centre.0,
            cross: centre.1,
            ha,
            hc,
        }
    }

    /// Signed distance to `other`, `0.0` or less when the boxes touch or overlap.
    fn gap(&self, other: &Box2) -> f64 {
        let da = (self.along - other.along).abs() - (self.ha + other.ha);
        let dc = (self.cross - other.cross).abs() - (self.hc + other.hc);
        match (da > 0.0, dc > 0.0) {
            (true, true) => fp::hypot(da, dc),
            (true, false) => da,
            (false, true) => dc,
            (false, false) => da.max(dc),
        }
    }

    /// `hit` for an overlap, pro rata `near` below `clearance`, otherwise zero.
    fn charge(&self, other: &Box2, hit: f64, near: f64, clearance: f64) -> f64 {
        let gap = self.gap(other);
        if gap <= 0.0 {
            hit
        } else if gap < clearance {
            near * (clearance - gap) / clearance
        } else {
            0.0
        }
    }
}

/// The node boxes and route segments a label has to miss, in a uniform bucket grid so a query
/// only scans nearby obstacles.
struct Obstacles {
    nodes: Vec<Box2>,
    /// `(edge, p, q)` per segment. `edge` lets a label skip its own polyline.
    segs: Vec<(usize, Pt, Pt)>,
    cell: f64,
    origin: Pt,
    cols: usize,
    rows: usize,
    /// Per cell: node indices then segment indices, the latter offset by `nodes.len()`.
    buckets: Vec<Vec<u32>>,
}

impl Obstacles {
    fn collect(scene: &LabelScene<'_>) -> Self {
        let nodes: Vec<Box2> = scene
            .nodes
            .iter()
            .map(|&(centre, (ha, hc))| Box2::new(centre, ha, hc))
            .collect();
        let mut segs = Vec::new();
        for (edge, route) in scene.routes.iter().enumerate() {
            for w in route.windows(2) {
                segs.push((edge, w[0], w[1]));
            }
        }
        for &(edge, route) in &scene.loops {
            for w in route.windows(2) {
                segs.push((edge, w[0], w[1]));
            }
        }

        let mut lo = (f64::INFINITY, f64::INFINITY);
        let mut hi = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        let mut extend = |p: Pt| {
            lo = (lo.0.min(p.0), lo.1.min(p.1));
            hi = (hi.0.max(p.0), hi.1.max(p.1));
        };
        for b in &nodes {
            extend((b.along - b.ha, b.cross - b.hc));
            extend((b.along + b.ha, b.cross + b.hc));
        }
        for &(_, p, q) in &segs {
            extend(p);
            extend(q);
        }
        if !lo.0.is_finite() {
            (lo, hi) = ((0.0, 0.0), (0.0, 0.0));
        }
        // About twice the mean obstacle spacing. Sizing by area keeps a wide, thin drawing from
        // collapsing into one row of buckets.
        let area = ((hi.0 - lo.0) * (hi.1 - lo.1)).max(1.0);
        let n = (nodes.len() + segs.len()).max(1) as f64;
        let cell = (2.0 * (area / n).sqrt()).clamp(GRID_CELL.0, GRID_CELL.1);
        let cols = (((hi.0 - lo.0) / cell).ceil() as usize + 1).min(4096);
        let rows = (((hi.1 - lo.1) / cell).ceil() as usize + 1).min(4096);
        let mut grid = Obstacles {
            nodes,
            segs,
            cell,
            origin: lo,
            cols,
            rows,
            buckets: vec![Vec::new(); cols * rows],
        };
        for i in 0..grid.nodes.len() {
            let b = grid.nodes[i];
            grid.insert(
                (b.along - b.ha, b.cross - b.hc),
                (b.along + b.ha, b.cross + b.hc),
                i as u32,
            );
        }
        let base = grid.nodes.len() as u32;
        for i in 0..grid.segs.len() {
            let (_, p, q) = grid.segs[i];
            grid.insert(
                (p.0.min(q.0), p.1.min(q.1)),
                (p.0.max(q.0), p.1.max(q.1)),
                base + i as u32,
            );
        }
        grid
    }

    /// Cell range covering `lo..hi`, clamped to the grid.
    fn range(&self, lo: Pt, hi: Pt) -> (usize, usize, usize, usize) {
        let ix = |v: f64, origin: f64, n: usize| {
            (((v - origin) / self.cell).floor().max(0.0) as usize).min(n.saturating_sub(1))
        };
        (
            ix(lo.0, self.origin.0, self.cols),
            ix(hi.0, self.origin.0, self.cols),
            ix(lo.1, self.origin.1, self.rows),
            ix(hi.1, self.origin.1, self.rows),
        )
    }

    fn insert(&mut self, lo: Pt, hi: Pt, item: u32) {
        let (c0, c1, r0, r1) = self.range(lo, hi);
        for r in r0..=r1 {
            for c in c0..=c1 {
                if let Some(bucket) = self.buckets.get_mut(r * self.cols + c) {
                    bucket.push(item);
                }
            }
        }
    }
}

/// Distance from segment `a..b` to `rect`, `0.0` when the segment enters it. Uses a Liang-Barsky
/// clip, then the distances between the segment ends and the box corners.
fn seg_box_distance(a: Pt, b: Pt, rect: &Box2) -> f64 {
    let (x0, x1) = (rect.along - rect.ha, rect.along + rect.ha);
    let (y0, y1) = (rect.cross - rect.hc, rect.cross + rect.hc);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    let mut outside = false;
    for &(pc, dc, lo, hi) in &[(a.0, b.0 - a.0, x0, x1), (a.1, b.1 - a.1, y0, y1)] {
        if dc.abs() < EPS {
            if pc < lo || pc > hi {
                outside = true;
                break;
            }
        } else {
            let (mut ta, mut tb) = ((lo - pc) / dc, (hi - pc) / dc);
            if ta > tb {
                std::mem::swap(&mut ta, &mut tb);
            }
            t0 = t0.max(ta);
            t1 = t1.min(tb);
        }
    }
    if !outside && t0 <= t1 {
        return 0.0;
    }
    let mut d = point_seg_dist_coarse((rect.along, rect.cross), a, b);
    for corner in [(x0, y0), (x1, y0), (x0, y1), (x1, y1)] {
        d = d.min(point_seg_dist_coarse(corner, a, b));
    }
    // Corner distances miss a segment that passes beside the box, so the ends are measured too.
    for p in [a, b] {
        d = d.min(Box2::new(p, 0.0, 0.0).gap(rect).max(0.0));
    }
    d
}

/// One position a label could take.
struct Candidate {
    rect: Box2,
    /// Position along its own polyline, for the mid-edge preference. The reserved gap centre
    /// reports `0.5`.
    fraction: f64,
}

/// The point at `frac` of `route`'s length and the unit normal of its segment.
fn sample(route: &[Pt], frac: f64) -> (Pt, Pt) {
    let lens: Vec<f64> = route
        .windows(2)
        .map(|w| f64::hypot(w[1].0 - w[0].0, w[1].1 - w[0].1))
        .collect();
    let total: f64 = lens.iter().sum();
    let target = total * frac.clamp(0.0, 1.0);
    let mut acc = 0.0;
    for (i, &len) in lens.iter().enumerate() {
        if acc + len >= target || i + 1 == lens.len() {
            let t = if len <= f64::EPSILON {
                0.0
            } else {
                ((target - acc) / len).clamp(0.0, 1.0)
            };
            let (a, b) = (route[i], route[i + 1]);
            let at = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            return (at, unit_normal(a, b));
        }
        acc += len;
    }
    (route[0], (0.0, 1.0))
}

/// Unit normal of `a -> b`, or the cross axis for a degenerate segment.
fn unit_normal(a: Pt, b: Pt) -> Pt {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = f64::hypot(dx, dy);
    if len <= f64::EPSILON {
        (0.0, 1.0)
    } else {
        (-dy / len, dx / len)
    }
}

/// Every position `edge`'s label is offered, in tie-break order.
fn candidates(
    scene: &LabelScene<'_>,
    edge: usize,
    ha: f64,
    hc: f64,
    reserved: Option<Pt>,
    o: &LayoutOptions,
) -> Vec<Candidate> {
    let route = scene.route(edge);
    let stroke = scene.thickness.get(edge).copied().unwrap_or(2.0) / 2.0;
    let per_point = 1 + 2 * o.tuning.labels.side_steps.len();
    let mut out = Vec::with_capacity((o.tuning.labels.fractions.len() + 1) * per_point + 1);
    let offer = |at: Pt, normal: Pt, fraction: f64, out: &mut Vec<Candidate>| {
        // Distance along `normal` that clears the line, its stroke and the clearance.
        let reach = normal.0.abs() * ha + normal.1.abs() * hc + stroke + o.labels.clearance;
        let mut sides = Vec::with_capacity(per_point);
        sides.push(0.0);
        for &step in &o.tuning.labels.side_steps {
            sides.push(step);
            sides.push(-step);
        }
        for side in sides {
            out.push(Candidate {
                rect: Box2::new(
                    (
                        at.0 + normal.0 * reach * side,
                        at.1 + normal.1 * reach * side,
                    ),
                    ha,
                    hc,
                ),
                fraction,
            });
        }
    };
    if route.len() >= 2 {
        for &frac in &o.tuning.labels.fractions {
            let (at, normal) = sample(route, frac);
            offer(at, normal, frac, &mut out);
        }
        // On an orthogonal route the longest segment is the run a label reads best on.
        if let Some((i, _)) = route
            .windows(2)
            .enumerate()
            .map(|(i, w)| (i, f64::hypot(w[1].0 - w[0].0, w[1].1 - w[0].1)))
            .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)))
        {
            let (a, b) = (route[i], route[i + 1]);
            offer(
                ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0),
                unit_normal(a, b),
                0.5,
                &mut out,
            );
        }
    }
    // The reserved gap centre is the fallback with guaranteed room.
    if let Some(at) = reserved {
        out.push(Candidate {
            rect: Box2::new(at, ha, hc),
            fraction: 0.5,
        });
    }
    out
}

/// Picks the cheapest candidate position for every labelled edge of `scene`.
pub(crate) fn place_labels(scene: &LabelScene<'_>, o: &LayoutOptions) -> Vec<Option<Pt>> {
    let n = scene.labels.len();
    let mut out = vec![None; n];
    if o.tuning.labels.rounds == 0 {
        return scene.reserved.to_vec();
    }
    // Biggest label first, because it has the fewest positions that fit. Edge index breaks ties.
    let mut todo: Vec<(usize, f64, f64)> = Vec::new();
    for (edge, half) in scene.labels.iter().enumerate() {
        let Some(&(ha, hc)) = half.as_ref() else {
            continue;
        };
        if !scene.route(edge).is_empty() || scene.reserved.get(edge).copied().flatten().is_some() {
            todo.push((edge, ha, hc));
        }
    }
    if todo.is_empty() {
        return out;
    }
    todo.sort_unstable_by_dyn(|a, b| (b.1 * b.2).total_cmp(&(a.1 * a.2)).then(a.0.cmp(&b.0)));

    let obstacles = Obstacles::collect(scene);
    let mut placed: Vec<Option<Box2>> = vec![None; n];
    let mut seen: Vec<u32> = vec![0; obstacles.nodes.len() + obstacles.segs.len()];
    let mut stamp: u32 = 0;
    for _ in 0..o.tuning.labels.rounds {
        for &(edge, ha, hc) in &todo {
            placed[edge] = None;
            let reserved = scene.reserved.get(edge).copied().flatten();
            let offers = candidates(scene, edge, ha, hc, reserved, o);
            let mut best: Option<(f64, Box2)> = None;
            for candidate in &offers {
                stamp += 1;
                // The charge stops once it passes the best so far.
                let cost = charge(
                    &candidate.rect,
                    candidate.fraction,
                    edge,
                    scene,
                    &obstacles,
                    &placed,
                    o,
                    &mut seen,
                    stamp,
                    best.map_or(f64::INFINITY, |(c, _)| c),
                );
                if best.is_none_or(|(c, _)| cost < c) {
                    best = Some((cost, candidate.rect));
                }
                if cost <= 0.0 {
                    break; // a clean position cannot be beaten, and earlier wins ties
                }
            }
            match best {
                Some((_, rect)) => {
                    placed[edge] = Some(rect);
                    out[edge] = Some((rect.along, rect.cross));
                }
                None => out[edge] = reserved,
            }
        }
    }
    out
}

/// What one candidate position costs.
#[allow(clippy::too_many_arguments)]
fn charge(
    rect: &Box2,
    fraction: f64,
    edge: usize,
    scene: &LabelScene<'_>,
    obstacles: &Obstacles,
    placed: &[Option<Box2>],
    o: &LayoutOptions,
    seen: &mut [u32],
    stamp: u32,
    limit: f64,
) -> f64 {
    let clearance = o.labels.clearance.max(MIN_CLEARANCE);
    let mut cost =
        OFF_CENTRE * (fraction - 0.5).abs() + DETACH * detachment(rect, scene.route(edge));
    if cost >= limit {
        return cost;
    }
    // Only obstacles in the cells the box plus clearance touches can score.
    let (c0, c1, r0, r1) = obstacles.range(
        (
            rect.along - rect.ha - clearance,
            rect.cross - rect.hc - clearance,
        ),
        (
            rect.along + rect.ha + clearance,
            rect.cross + rect.hc + clearance,
        ),
    );
    let n_nodes = obstacles.nodes.len() as u32;
    for r in r0..=r1 {
        for c in c0..=c1 {
            let Some(bucket) = obstacles.buckets.get(r * obstacles.cols + c) else {
                continue;
            };
            for &item in bucket {
                match seen.get_mut(item as usize) {
                    Some(slot) if *slot != stamp => *slot = stamp,
                    _ => continue,
                }
                if item < n_nodes {
                    cost += rect.charge(
                        &obstacles.nodes[item as usize],
                        NODE_HIT,
                        NODE_NEAR,
                        clearance,
                    );
                } else {
                    let (owner, p, q) = obstacles.segs[(item - n_nodes) as usize];
                    if owner == edge {
                        continue; // a label sits on its own edge by design
                    }
                    let d = seg_box_distance(p, q, rect);
                    cost += if d <= 0.0 {
                        EDGE_HIT
                    } else if d < clearance {
                        EDGE_NEAR * (clearance - d) / clearance
                    } else {
                        0.0
                    };
                }
                if cost >= limit {
                    return cost;
                }
            }
        }
    }
    for (other, box2) in placed.iter().enumerate() {
        if other == edge {
            continue;
        }
        if let Some(other) = box2 {
            cost += rect.charge(other, LABEL_HIT, LABEL_NEAR, clearance);
            if cost >= limit {
                return cost;
            }
        }
    }
    cost
}

/// How far the label's centre is from its own polyline. Charged even on a clean position, because
/// a label that cannot be tied to its edge is the worst label defect.
fn detachment(rect: &Box2, route: &[Pt]) -> f64 {
    if route.len() < 2 {
        return 0.0;
    }
    route
        .windows(2)
        .map(|w| point_seg_dist_coarse((rect.along, rect.cross), w[0], w[1]))
        .fold(f64::INFINITY, f64::min)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::shape::NodeShape;
    use crate::layered::prepare::{LayeredInput, LayeredInputNode};
    use crate::options::{Direction, LabelPlacement, LabelReservation};

    /// A chain of `n` boxes with edge `i` from node `i` to `i + 1`, plus one long edge from the
    /// first node to the last. Ranks are the node indices.
    fn chain_state(n: usize, direction: Direction) -> LayeredState {
        let node = || LayeredInputNode {
            width: 140.0,
            height: 44.0,
            shape: NodeShape::Box,
            constraint: None,
            category: None,
            clear_after: 0.0,
            clear_before: 0.0,
            port_sides: None,
            lane_key: None,
            along_before: 0.0,
            along_after: 0.0,
        };
        let mut edges: Vec<(usize, usize)> = (0..n - 1).map(|i| (i, i + 1)).collect();
        edges.push((0, n - 1));
        let input = LayeredInput {
            nodes: (0..n).map(|_| node()).collect(),
            edges,
            weights: vec![],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: LayoutOptions {
                direction,
                ..LayoutOptions::layered_default()
            },
        };
        let mut st = LayeredState::new(&input);
        for (i, nd) in st.nodes.iter_mut().enumerate() {
            nd.rank = i as i32;
            nd.cross = 0.0;
        }
        st.rebuild_layers();
        for i in 0..n - 1 {
            st.chains.insert(i, vec![i, i + 1]);
        }
        let mut long: Vec<usize> = vec![0];
        for r in 1..n - 1 {
            let d = st.add_dummy(r as i32);
            st.nodes[d].cross = 300.0;
            long.push(d);
        }
        long.push(n - 1);
        st.chains.insert(n - 1, long);
        st.rebuild_layers();
        st
    }

    /// Under `LabelReservation::Always`, every labelled edge reserves in its middle gap only, and
    /// no rank is widened.
    #[test]
    fn each_label_widens_one_gap_only() {
        let mut o = LayoutOptions::layered_default();
        o.labels.reservation = LabelReservation::Always;
        let st = chain_state(4, Direction::LeftRight);
        // Edges 0, 1 and 2 span gaps 0, 1 and 2. Edge 3 is the long one, in gap 1.
        let sizes = vec![(200.0, 14.0), (0.0, 0.0), (0.0, 0.0), (500.0, 14.0)];
        let gap = reserve_label_gaps(&st, &sizes, &o);
        assert_eq!(gap[0], 200.0 + o.spacing.edge_label_margin);
        assert_eq!(gap[1], 500.0 + o.spacing.edge_label_margin);
        assert_eq!(gap[2], o.spacing.layer_gap);
        assert!(st.nodes.iter().skip(4).all(|nd| nd.w_along == 0.0));
    }

    /// Top-to-bottom swaps the axes, so the label's height has to fit in the gap.
    #[test]
    fn the_gap_reserves_the_layer_axis_extent() {
        let mut o = LayoutOptions::layered_default();
        o.labels.reservation = LabelReservation::Always;
        let st = chain_state(3, Direction::TopBottom);
        let gap = reserve_label_gaps(&st, &[(500.0, 100.0); 3], &o);
        assert_eq!(gap[0], 100.0 + o.spacing.edge_label_margin);
        // A label shorter than the base gap does not shrink it.
        let gap = reserve_label_gaps(&st, &[(500.0, 14.0); 3], &o);
        assert_eq!(gap[0], o.spacing.layer_gap);
    }

    /// The default reservation only widens a gap for a label taller than `node_gap` allows.
    #[test]
    fn a_slim_label_no_longer_widens_its_gap() {
        let o = LayoutOptions::layered_default();
        assert_eq!(o.labels.reservation, LabelReservation::WhenNeeded);
        let st = chain_state(3, Direction::LeftRight);
        let gap = reserve_label_gaps(&st, &[(500.0, 14.0); 3], &o);
        assert_eq!(gap[0], o.spacing.layer_gap);
        let fat = o.spacing.node_gap + 1.0;
        let gap = reserve_label_gaps(&st, &[(500.0, fat); 3], &o);
        assert_eq!(gap[0], 500.0 + o.spacing.edge_label_margin);
        let mut never = o;
        never.labels.reservation = LabelReservation::Never;
        let gap = reserve_label_gaps(&st, &[(500.0, fat); 3], &never);
        assert_eq!(gap[0], never.spacing.layer_gap);
    }

    /// Gap-centre placement stacks labels that cross one gap at the same cross position.
    #[test]
    fn labels_sharing_a_gap_stack_on_the_cross_axis() {
        let mut o = LayoutOptions::layered_default();
        o.labels.reservation = LabelReservation::Always;
        o.labels.placement = LabelPlacement::GapCentre;
        let mut st = chain_state(3, Direction::LeftRight);
        // Three more edges across gap 0, all at cross 0 like the first.
        for arc in 2..5 {
            st.chains.insert(arc, vec![0, 1]);
        }
        st.routes = vec![vec![]; 5];
        st.thickness = vec![2.0; 5];
        // Arc 1 crosses gap 1 and stays unlabelled.
        let mut sizes = vec![(200.0, 40.0); 5];
        sizes[1] = (0.0, 0.0);
        let gap = reserve_label_gaps(&st, &sizes, &o);
        let anchors = place_edge_labels(&st, &sizes, &gap, &o);
        let mut placed: Vec<f64> = [0usize, 2, 3, 4]
            .iter()
            .map(|&a| anchors[a].unwrap().1)
            .collect();
        placed.sort_unstable_by_dyn(f64::total_cmp);
        for w in placed.windows(2) {
            assert!(
                w[1] - w[0] >= 40.0 + o.spacing.edge_label_stack_gap - 1e-9,
                "{placed:?} overlap"
            );
        }
        let along = anchors[0].unwrap().0;
        assert!([2, 3, 4].iter().all(|&a| anchors[a].unwrap().0 == along));
        assert!(anchors[1].is_none());
    }

    /// A label whose mid-edge position sits on a node box moves off it but stays near its edge.
    #[test]
    fn a_label_is_moved_off_a_node_box() {
        let o = LayoutOptions::layered_default();
        let mut st = chain_state(3, Direction::LeftRight);
        st.thickness = vec![2.0; 3];
        // A third box sits on the middle of the route between the other two.
        for (i, along) in [0.0, 400.0, 200.0].into_iter().enumerate() {
            st.nodes[i].along = along;
            st.nodes[i].cross = 0.0;
        }
        st.routes = vec![vec![(70.0, 0.0), (330.0, 0.0)], vec![], vec![]];
        let sizes = vec![(40.0, 16.0), (0.0, 0.0), (0.0, 0.0)];
        let gap = reserve_label_gaps(&st, &sizes, &o);
        let anchor = place_edge_labels(&st, &sizes, &gap, &o)[0].unwrap();
        let (ha, hc) = (20.0, 8.0);
        let label = Box2::new(anchor, ha, hc);
        for nd in st.nodes.iter().take(3) {
            let box2 = Box2::new((nd.along, nd.cross), nd.w_along / 2.0, nd.w_cross / 2.0);
            assert!(label.gap(&box2) > 0.0, "{anchor:?} sits on a node box");
        }
        assert!(detachment(&label, &st.routes[0]) <= hc + o.labels.clearance + 1.0 + 1e-9);
    }

    /// Two labels competing for the same free space do not overlap.
    #[test]
    fn two_labels_over_one_free_spot_separate() {
        let o = LayoutOptions::layered_default();
        let mut st = chain_state(3, Direction::LeftRight);
        st.thickness = vec![2.0; 3];
        for (i, cross) in [0.0, 0.0, 600.0].into_iter().enumerate() {
            st.nodes[i].along = if i == 0 { 0.0 } else { 400.0 };
            st.nodes[i].cross = cross;
        }
        // Parallel edges 4px apart, so the mid-edge positions coincide.
        st.routes = vec![
            vec![(70.0, 0.0), (330.0, 0.0)],
            vec![(70.0, 4.0), (330.0, 4.0)],
            vec![],
        ];
        let sizes = vec![(60.0, 16.0), (60.0, 16.0), (0.0, 0.0)];
        let gap = reserve_label_gaps(&st, &sizes, &o);
        let anchors = place_edge_labels(&st, &sizes, &gap, &o);
        let (a, b) = (anchors[0].unwrap(), anchors[1].unwrap());
        let (ra, rb) = (Box2::new(a, 30.0, 8.0), Box2::new(b, 30.0, 8.0));
        assert!(ra.gap(&rb) > 0.0, "{a:?} and {b:?} overlap");
    }

    /// Segment-to-box distance for a segment through, beside and short of the box.
    #[test]
    fn segment_to_box_distance_is_the_geometric_one() {
        let rect = Box2::new((0.0, 0.0), 10.0, 5.0);
        assert_eq!(seg_box_distance((-20.0, 0.0), (20.0, 0.0), &rect), 0.0);
        assert_eq!(seg_box_distance((-20.0, 15.0), (20.0, 15.0), &rect), 10.0);
        assert_eq!(seg_box_distance((30.0, 0.0), (50.0, 0.0), &rect), 20.0);
        // Diagonally off a corner, the distance is to the corner.
        let d = seg_box_distance((13.0, 9.0), (40.0, 40.0), &rect);
        assert!((d - f64::hypot(3.0, 4.0)).abs() < 1e-9, "{d}");
    }
}
