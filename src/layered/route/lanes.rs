//! A bundle's lanes, drawn off its finished corridor. Each lane has its own ports from
//! [`super::spread`] and runs between them along the corridor at the bundle's tight spacing.

use super::spread::LaneOffsets;
use super::{PortMap, RouteCtx};
use crate::geometry::polyline::{offset_polyline_segments, simplify_collinear};
use crate::geometry::{Boxes, Side};
use crate::layered::ports::{port_offset_limit, project_port};

/// Which way along a border a lane's right-of-travel offset runs, given the face and whether the
/// corridor leaves or arrives. `+1` means further right of travel is further along the border.
pub(super) fn lane_dir(side: Side, is_source: bool) -> f64 {
    let out = match side {
        Side::Right | Side::Top => 1.0,
        Side::Left | Side::Bottom => -1.0,
    };
    if is_source {
        out
    } else {
        -out
    }
}

/// Lane offsets inside the corridor, half a stroke each plus [`crate::Spacing::bundle_lane_gap`]
/// apart and centred. Staying this tight keeps a riser off the neighbouring gutter lane.
pub(super) fn tight_lane_pattern(strokes: &[f64], gap: f64) -> Vec<f64> {
    let mut pos = Vec::with_capacity(strokes.len());
    let mut at = 0.0;
    for (i, &stroke) in strokes.iter().enumerate() {
        if i > 0 {
            at += (strokes[i - 1] + stroke) / 2.0 + gap;
        }
        pos.push(at);
    }
    let centre = at / 2.0;
    for p in &mut pos {
        *p -= centre;
    }
    pos
}

pub(super) fn draw_lanes(
    ctx: &RouteCtx,
    port: &PortMap,
    lane_offsets: &LaneOffsets,
    routes: &[Vec<(f64, f64)>],
) -> Vec<Vec<Vec<(f64, f64)>>> {
    let mut lanes: Vec<Vec<Vec<(f64, f64)>>> = vec![vec![]; ctx.st.routes.len()];
    for (arc, chain) in ctx.ordered() {
        let (Some(s_off), Some(t_off)) = (
            lane_offsets.get(&(arc, true)),
            lane_offsets.get(&(arc, false)),
        ) else {
            continue;
        };
        let (Some(ps), Some(pt)) = (port.get(&(arc, true)), port.get(&(arc, false))) else {
            continue;
        };
        let (Some(&lo), Some(&hi)) = (chain.first(), chain.last()) else {
            continue;
        };
        let tight = tight_lane_pattern(ctx.lane_strokes(arc), ctx.o.spacing.bundle_lane_gap);
        lanes[arc] = lane_routes(
            &routes[arc],
            (lo, hi),
            (ps.side, pt.side),
            (s_off, t_off),
            &tight,
            ctx.b,
            ctx.stub_len,
        );
    }
    lanes
}

/// The corridor copied sideways once per lane, each end on the lane's own port. End segments sit
/// at the `pins` offsets and the rest at `tight`. The offset changes at turns, costing no bend.
pub(super) fn lane_routes(
    corridor: &[(f64, f64)],
    ends: (usize, usize),
    sides: (Side, Side),
    pins: (&[f64], &[f64]),
    tight: &[f64],
    b: &Boxes,
    stub_len: f64,
) -> Vec<Vec<(f64, f64)>> {
    let n = corridor.len();
    let lanes = tight.len();
    if n < 2 || lanes < 2 || pins.0.len() != lanes || pins.1.len() != lanes {
        return vec![];
    }
    // Re-centre the fan so its outermost lane fits the border. Two lanes clamped onto one point
    // would draw as one edge.
    let end_at = |p: (f64, f64), node: usize, side: Side, pin: &[f64]| -> (f64, f64) {
        let half = pin.iter().fold(0.0f64, |m, d| m.max(d.abs()));
        let raw = if side.horizontal() {
            p.1 - b.cy[node]
        } else {
            p.0 - b.cx[node]
        };
        let room = (port_offset_limit(node, side, b) - half).max(0.0);
        (raw, raw.clamp(-room, room))
    };
    let (corner_s, centre_s) = end_at(corridor[0], ends.0, sides.0, pins.0);
    let (corner_t, centre_t) = end_at(corridor[n - 1], ends.1, sides.1, pins.1);
    let (dir_s, dir_t) = (lane_dir(sides.0, true), lane_dir(sides.1, false));
    // An end's offset carries through every leading segment parallel to its end segment.
    let family = |i: usize| -> bool {
        (corridor[i + 1].0 - corridor[i].0).abs() >= (corridor[i + 1].1 - corridor[i].1).abs()
    };
    let segs = n - 1;
    let head = (0..segs).take_while(|&i| family(i) == family(0)).count();
    let tail = segs
        - (0..segs)
            .rev()
            .take_while(|&i| family(i) == family(segs - 1))
            .count();
    (0..lanes)
        .map(|lane| {
            let mut d_s = dir_s * (centre_s + dir_s * pins.0[lane] - corner_s);
            let mut d_t = dir_t * (centre_t + dir_t * pins.1[lane] - corner_t);
            // A straight run is one segment for both ends, so take the offset both borders hold.
            if head > tail {
                let shared = if d_t.abs() < d_s.abs() { d_t } else { d_s };
                (d_s, d_t) = (shared, shared);
            }
            let ps = project_port(ends.0, sides.0, corner_s + dir_s * d_s, b, stub_len);
            let pt = project_port(ends.1, sides.1, corner_t + dir_t * d_t, b, stub_len);
            let d: Vec<f64> = (0..segs)
                .map(|i| match (i < head, i >= tail) {
                    (true, _) => d_s,
                    (_, true) => d_t,
                    _ => tight[lane],
                })
                .collect();
            let mut pts = offset_polyline_segments(corridor, &d);
            // The offset copy ends on the box border, but an ellipse's outline is further in.
            let last = pts.len().saturating_sub(1);
            pts[0] = ps.anchor;
            pts[last] = pt.anchor;
            simplify_collinear(pts)
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::route;
    use super::*;
    use crate::geometry::fp;
    use crate::geometry::shape::NodeShape;
    use crate::layered::prepare::{LayeredInput, LayeredInputNode};
    use crate::layered::state::{LArc, LayeredState};
    use crate::options::{LayoutOptions, Routing};
    use crate::sort::SortDyn;

    /// A bundle of `lanes` arcs spanning `span` layers, in flow space with roomy borders.
    fn bundled(lanes: usize, span: usize, height: f64) -> (LayeredState, LayoutOptions) {
        let node = |h: f64| LayeredInputNode {
            width: 120.0,
            height: h,
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
        let input = LayeredInput {
            nodes: vec![node(height), node(height)],
            edges: vec![(0, 1)],
            weights: vec![],
            thickness: vec![],
            edge_label_sizes: vec![],
            seed: vec![],
            pinned: vec![],
            self_loops: vec![],
            ranked_backwards: Vec::new(),
            options: LayoutOptions {
                direction: crate::options::Direction::LeftRight,
                ..LayoutOptions::layered_default()
            },
        };
        let mut st = LayeredState::new(&input);
        // As in `bundle::bundle_parallel_arcs`, the corridor's stroke is the fan's width.
        st.lanes = vec![vec![2.0; lanes]];
        st.thickness = vec![2.0 * lanes as f64];
        let mut chain = vec![0usize];
        for k in 0..span.saturating_sub(1) {
            let d = st.add_dummy(1 + k as i32);
            st.nodes[d].along = 300.0 + 300.0 * k as f64;
            // Enough offset for a riser, too little to make the router pick a perpendicular side.
            st.nodes[d].cross = 60.0;
            chain.push(d);
        }
        chain.push(1);
        st.nodes[1].rank = span as i32;
        st.nodes[0].along = 60.0;
        st.nodes[0].cross = 0.0;
        st.nodes[1].along = 300.0 * span as f64 + 60.0;
        st.nodes[1].cross = 0.0;
        st.arcs = chain
            .windows(2)
            .map(|w| LArc {
                from: w[0],
                to: w[1],
                edge: 0,
                reversed: false,
            })
            .collect();
        st.chains.insert(0, chain.clone());
        st.layers = (0..=span)
            .map(|rank| {
                chain
                    .iter()
                    .copied()
                    .filter(|&n| st.nodes[n].rank == rank as i32)
                    .collect()
            })
            .collect();
        st.reindex();
        let o = LayoutOptions {
            routing: Routing::default(),
            ..st_options(&st)
        };
        (st, o)
    }

    fn st_options(_st: &LayeredState) -> LayoutOptions {
        LayoutOptions {
            direction: crate::options::Direction::LeftRight,
            ..LayoutOptions::layered_default()
        }
    }

    fn axis_aligned(p: (f64, f64), q: (f64, f64)) -> bool {
        (p.0 - q.0).abs() < 0.5 || (p.1 - q.1).abs() < 0.5
    }

    #[test]
    fn lane_dir_follows_the_face_the_corridor_uses() {
        assert_eq!(lane_dir(Side::Right, true), 1.0);
        assert_eq!(lane_dir(Side::Left, true), -1.0);
        assert_eq!(lane_dir(Side::Top, true), 1.0);
        assert_eq!(lane_dir(Side::Bottom, true), -1.0);
        for side in [Side::Right, Side::Left, Side::Top, Side::Bottom] {
            assert_eq!(lane_dir(side, false), -lane_dir(side, true));
        }
    }

    #[test]
    fn tight_lane_pattern_centres_the_lanes_on_the_corridor() {
        assert_eq!(tight_lane_pattern(&[2.0, 2.0], 8.0), vec![-5.0, 5.0]);
        assert_eq!(
            tight_lane_pattern(&[2.0, 2.0, 2.0], 8.0),
            vec![-10.0, 0.0, 10.0]
        );
        assert_eq!(tight_lane_pattern(&[10.0, 2.0], 8.0), vec![-7.0, 7.0]);
        assert!(tight_lane_pattern(&[2.0], 8.0) == vec![0.0]);
    }

    /// Each lane gets its own port, a port gap apart, with an axis-aligned end segment.
    #[test]
    fn bundle_lanes_get_a_port_each_a_port_gap_apart() {
        let (mut st, o) = bundled(3, 1, 120.0);
        route(&mut st, &o).unwrap();
        let lanes = &st.lane_routes[0];
        assert_eq!(lanes.len(), 3, "one polyline per lane");
        let mut ends: Vec<(f64, f64)> = vec![];
        for lane in lanes {
            assert!(lane.len() >= 2, "{lane:?}");
            let n = lane.len();
            assert!(
                axis_aligned(lane[0], lane[1]) && axis_aligned(lane[n - 2], lane[n - 1]),
                "end segment not axis-aligned: {lane:?}"
            );
            assert!(
                (lane[0].0 - (st.nodes[0].along + 60.0)).abs() < 1e-6,
                "start off the source border: {:?}",
                lane[0]
            );
            assert!(
                (lane[n - 1].0 - (st.nodes[1].along - 60.0)).abs() < 1e-6,
                "end off the target border: {:?}",
                lane[n - 1]
            );
            ends.push(lane[0]);
        }
        ends.sort_by_dyn(|a, b| a.1.total_cmp(&b.1));
        for pair in ends.windows(2) {
            assert!(
                pair[1].1 - pair[0].1 >= o.spacing.port_gap - 1e-6,
                "two lanes closer than a port gap: {ends:?}"
            );
        }
    }

    /// Ports fan out a port gap apart while interior runs keep the tight lane gap, with no extra
    /// bends.
    #[test]
    fn bundle_lanes_keep_the_corridor_tight_between_the_ports() {
        let (mut st, mut o) = bundled(2, 3, 120.0);
        // The default would flatten the two steps into one and leave no interior run to measure.
        o.tuning.routing.step_flatten = 12.0;
        route(&mut st, &o).unwrap();
        let lanes = &st.lane_routes[0];
        assert_eq!(lanes.len(), 2);
        let corridor = &st.routes[0];
        for lane in lanes {
            assert_eq!(
                lane.len(),
                corridor.len(),
                "a lane bends where its corridor does and nowhere else: {lane:?} vs {corridor:?}"
            );
        }
        let port_gap = fp::hypot(lanes[1][0].0 - lanes[0][0].0, lanes[1][0].1 - lanes[0][0].1);
        assert!(
            port_gap >= o.spacing.port_gap - 1e-6,
            "ports too close: {port_gap}"
        );
        // An interior run carries neither port.
        let channel = |lane: &[(f64, f64)]| {
            let last = lane.len().saturating_sub(2);
            lane.windows(2)
                .enumerate()
                .filter(|(i, w)| {
                    *i > 0
                        && *i < last
                        && (w[0].1 - w[1].1).abs() < 0.5
                        && (w[0].0 - w[1].0).abs() > 50.0
                })
                .map(|(_, w)| w[0].1)
                .next()
        };
        let (a, b) = (channel(&lanes[0]), channel(&lanes[1]));
        let (Some(a), Some(b)) = (a, b) else {
            panic!("no channel run: {lanes:?}");
        };
        assert!(
            ((a - b).abs() - (2.0 + o.spacing.bundle_lane_gap)).abs() < 1e-6,
            "channel runs {a} and {b} are not a lane gap apart"
        );
    }

    /// A full border shrinks the fan instead of letting `project_port` clamp a port, so lanes
    /// may share a line but no end segment slants.
    #[test]
    fn a_full_border_cuts_the_fan_back_instead_of_slanting_it() {
        let (mut st, o) = bundled(6, 1, 20.0);
        route(&mut st, &o).unwrap();
        let lanes = &st.lane_routes[0];
        assert_eq!(lanes.len(), 6);
        for lane in lanes {
            let n = lane.len();
            assert!(
                axis_aligned(lane[0], lane[1]) && axis_aligned(lane[n - 2], lane[n - 1]),
                "end segment not axis-aligned on a full border: {lane:?}"
            );
            assert!(
                (lane[0].1 - st.nodes[0].cross).abs() <= 10.0 + 1e-6,
                "port off the border span: {:?}",
                lane[0]
            );
        }
    }

    #[test]
    fn a_plain_arc_has_no_lanes() {
        let (mut st, o) = bundled(1, 1, 120.0);
        st.lanes = vec![];
        st.thickness = vec![2.0];
        route(&mut st, &o).unwrap();
        assert!(st.lane_routes[0].is_empty());
        assert!(st.routes[0].len() >= 2);
    }
}
