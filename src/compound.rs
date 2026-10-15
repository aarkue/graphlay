//! Compound (nested) layout, built bottom-up.
//!
//! Each compound is laid out as a flat drawing of its direct children, deepest first, and the
//! level above treats it as one node of that size. An edge to a deeper end is cut at each compound
//! border into pieces that meet at a boundary node on the border, then joined into one polyline.

use std::collections::BTreeMap;

use crate::error::LayoutError;
use crate::geometry::polyline::simplify_collinear;
use crate::geometry::{Point, Rect, Side, Size};
use crate::graph::{Edge, EdgeId, Graph, Node, NodeId, PortHint, RankConstraint};
use crate::layout::{EdgeRoute, Lane, Layout, LayoutStats, Placement, RouteKind};
use crate::options::{Direction, LayoutOptions};

/// Size of a boundary node, a valid box that still reads as a point.
const BOUNDARY: f64 = 2.0;
/// Share of the half-border a boundary node may sit from its middle, inside the router's port
/// limit.
const PORT_REACH: f64 = 0.9;
/// Two pieces of one edge whose ends are this close (px) meet.
const JOIN_EPS: f64 = 0.5;

pub(crate) fn has_compounds(g: &Graph) -> bool {
    g.nodes.iter().any(|n| n.parent.is_some())
}

/// Checks that every `parent` names another node and that parent chains have no cycle.
pub(crate) fn validate(g: &Graph) -> Result<(), LayoutError> {
    let n = g.nodes.len();
    for (i, nd) in g.nodes.iter().enumerate() {
        let Some(p) = nd.parent else { continue };
        if p.index() >= n || p.index() == i {
            return Err(LayoutError::InvalidParent {
                node: NodeId(i as u32),
            });
        }
        // A chain longer than the graph has nodes has gone round a cycle.
        let mut at = p.index();
        for _ in 0..n {
            match g.nodes[at].parent {
                Some(q) if q.index() == i => {
                    return Err(LayoutError::InvalidParent {
                        node: NodeId(i as u32),
                    })
                }
                Some(q) if q.index() < n => at = q.index(),
                _ => break,
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum End {
    Node(usize),
    /// The border the edge enters the level's compound through (its backward face).
    In,
    /// The border the edge leaves the level's compound through (its forward face).
    Out,
}

/// One piece of an edge, drawn at `level` (`None` for the top level).
#[derive(Clone, Copy, Debug)]
struct Piece {
    level: Option<usize>,
    from: End,
    to: End,
}

/// Moves each boundary node level with the port its piece reaches, where the straight run between
/// them is clear, so the piece does not detour and widen the compound.
fn snap_boundaries(
    drawn: &mut Layout,
    sub: &Graph,
    boundary: &BTreeMap<(usize, bool), usize>,
    flow_along_x: bool,
) {
    let split = |p: Point| if flow_along_x { (p.x, p.y) } else { (p.y, p.x) };
    let join = |along: f64, cross: f64| {
        if flow_along_x {
            Point::new(along, cross)
        } else {
            Point::new(cross, along)
        }
    };
    let hits = |a: Point, b: Point, pl: &Placement| {
        let (x0, x1) = (a.x.min(b.x), a.x.max(b.x));
        let (y0, y1) = (a.y.min(b.y), a.y.max(b.y));
        let (hw, hh) = (pl.size.width / 2.0 - 0.5, pl.size.height / 2.0 - 0.5);
        x0 < pl.center.x + hw
            && x1 > pl.center.x - hw
            && y0 < pl.center.y + hh
            && y1 > pl.center.y - hh
    };
    for (&(_, out), &b) in boundary {
        let Some((se, e)) = sub.edges.iter().enumerate().find(|(_, e)| {
            if out {
                e.target.index() == b
            } else {
                e.source.index() == b
            }
        }) else {
            continue;
        };
        let far = if out {
            e.source.index()
        } else {
            e.target.index()
        };
        let points = &drawn.edges[se].points;
        let (Some(&near_end), Some(&far_end)) = (
            if out { points.last() } else { points.first() },
            if out { points.first() } else { points.last() },
        ) else {
            continue;
        };
        let (far_along, far_cross) = split(far_end);
        let (near_along, near_cross) = split(near_end);
        if (far_cross - near_cross).abs() < JOIN_EPS {
            continue;
        }
        // Only an end on the far node's facing border can be reached by a straight run.
        let far_box = drawn.nodes[far];
        let (fc_along, _) = split(far_box.center);
        let half_along = if flow_along_x {
            far_box.size.width
        } else {
            far_box.size.height
        } / 2.0;
        let facing = if out {
            fc_along + half_along
        } else {
            fc_along - half_along
        };
        if (far_along - facing).abs() > JOIN_EPS {
            continue;
        }
        let start = join(near_along, far_cross);
        let clear = drawn
            .nodes
            .iter()
            .enumerate()
            .all(|(i, pl)| i == b || i == far || !hits(start, far_end, pl));
        if !clear {
            continue;
        }
        let (b_along, _) = split(drawn.nodes[b].center);
        drawn.nodes[b].center = join(b_along, far_cross);
        let route = &mut drawn.edges[se];
        route.points = if out {
            vec![far_end, start]
        } else {
            vec![start, far_end]
        };
        if route.label_anchor.is_some() {
            route.label_anchor = Some(join((near_along + far_along) / 2.0, far_cross));
        }
    }
}

/// The piece that carries the edge's label and whose direction [`Layout::reversed`] reports.
fn main_piece(list: &[Piece]) -> usize {
    list.iter()
        .position(|p| matches!((p.from, p.to), (End::Node(_), End::Node(_))))
        .unwrap_or_else(|| {
            // From a container the first piece is at its level, into one the last piece is.
            if matches!(list.first().map(|p| p.from), Some(End::In)) {
                0
            } else {
                list.len().saturating_sub(1)
            }
        })
}

struct Level {
    layout: Layout,
    index: BTreeMap<usize, usize>,
    edge: BTreeMap<(usize, usize), usize>,
    /// Unused for the top level.
    frame: Rect,
}

pub(crate) fn layout(
    g: &Graph,
    o: &LayoutOptions,
    layout_flat: impl Fn(&Graph, &LayoutOptions) -> Result<Layout, LayoutError>,
) -> Result<Layout, LayoutError> {
    let n = g.nodes.len();
    let parent: Vec<Option<usize>> = g
        .nodes
        .iter()
        .map(|nd| nd.parent.map(NodeId::index))
        .collect();
    let mut children: Vec<Vec<usize>> = vec![vec![]; n];
    for (i, p) in parent.iter().enumerate() {
        if let Some(p) = p {
            children[*p].push(i);
        }
    }
    let ancestors = |mut x: usize| {
        let mut chain = vec![x];
        while let Some(p) = parent[x] {
            chain.push(p);
            x = p;
        }
        chain
    };

    let mut pieces: Vec<Vec<Piece>> = Vec::with_capacity(g.edges.len());
    for e in &g.edges {
        let (u, v) = (e.source.index(), e.target.index());
        let (au, av) = (ancestors(u), ancestors(v));
        let mut list = vec![];
        if u == v {
            list.push(Piece {
                level: parent[u],
                from: End::Node(u),
                to: End::Node(v),
            });
        } else if let Some(k) = av.iter().position(|&x| x == u) {
            for w in av[..=k].windows(2).rev() {
                list.push(Piece {
                    level: Some(w[1]),
                    from: End::In,
                    to: End::Node(w[0]),
                });
            }
        } else if let Some(k) = au.iter().position(|&x| x == v) {
            for w in au[..=k].windows(2) {
                list.push(Piece {
                    level: Some(w[1]),
                    from: End::Node(w[0]),
                    to: End::Out,
                });
            }
        } else {
            let lca = au.iter().copied().find(|x| av.contains(x));
            let iu = au.iter().position(|&x| Some(x) == lca).unwrap_or(au.len());
            let iv = av.iter().position(|&x| Some(x) == lca).unwrap_or(av.len());
            for w in au[..iu].windows(2) {
                list.push(Piece {
                    level: Some(w[1]),
                    from: End::Node(w[0]),
                    to: End::Out,
                });
            }
            list.push(Piece {
                level: lca,
                from: End::Node(au[iu - 1]),
                to: End::Node(av[iv - 1]),
            });
            for w in av[..iv].windows(2).rev() {
                list.push(Piece {
                    level: Some(w[1]),
                    from: End::In,
                    to: End::Node(w[0]),
                });
            }
        }
        pieces.push(list);
    }

    let mut post: Vec<usize> = vec![];
    let mut stack: Vec<(usize, bool)> = (0..n)
        .rev()
        .filter(|&i| parent[i].is_none() && !children[i].is_empty())
        .map(|i| (i, false))
        .collect();
    while let Some((c, done)) = stack.pop() {
        if done {
            post.push(c);
            continue;
        }
        stack.push((c, true));
        for &k in children[c].iter().rev() {
            if !children[k].is_empty() {
                stack.push((k, false));
            }
        }
    }

    // Inner levels have no rows to wrap into.
    let mut inner = *o;
    inner.wrap.viewport_aspect = 0.0;
    let flow_along_x = o.direction == Direction::LeftRight;
    let (in_side, out_side) = if flow_along_x {
        (Side::Left, Side::Right)
    } else {
        (Side::Top, Side::Bottom)
    };

    // A compound's `size` is its children's box. The header sits above it as a margin while the
    // level above is laid out, and goes back into the drawn box at the end.
    let header = o.spacing.compound_header;
    let mut size: Vec<Size> = g.nodes.iter().map(|nd| nd.size).collect();
    let header_margin = |nd: &Node| {
        let mut m = nd.margin;
        if flow_along_x {
            m.cross_neg += header;
        } else {
            m.along_neg += header;
        }
        m
    };
    let mut port_t: BTreeMap<(usize, usize, bool), f64> = BTreeMap::new();
    let mut levels: BTreeMap<Option<usize>, Level> = BTreeMap::new();
    for level in post.iter().copied().map(Some).chain(std::iter::once(None)) {
        let members: Vec<usize> = match level {
            Some(c) => children[c].clone(),
            None => (0..n).filter(|&i| parent[i].is_none()).collect(),
        };
        let mut sub = Graph::new();
        let mut index = BTreeMap::new();
        for &m in &members {
            index.insert(m, sub.nodes.len());
            let margin = if children[m].is_empty() {
                g.nodes[m].margin
            } else {
                header_margin(&g.nodes[m])
            };
            sub.nodes.push(Node {
                size: size[m],
                margin,
                parent: None,
                position: None,
                pinned: false,
                ..g.nodes[m].clone()
            });
        }
        let mut edge = BTreeMap::new();
        let mut boundary = BTreeMap::new();
        for (e, list) in pieces.iter().enumerate() {
            for (k, piece) in list.iter().enumerate().filter(|(_, p)| p.level == level) {
                let mut end = |end: End, is_source: bool| -> (usize, Option<PortHint>) {
                    match end {
                        End::Node(m) => {
                            let deeper = if is_source {
                                list.get(k.wrapping_sub(1)).filter(|_| k > 0)
                            } else {
                                list.get(k + 1)
                            };
                            let hint = deeper.and_then(|_| {
                                let t = port_t.get(&(m, e, is_source))?;
                                Some(PortHint::Offset {
                                    side: if is_source { out_side } else { in_side },
                                    t: *t,
                                })
                            });
                            (index[&m], hint)
                        }
                        End::In | End::Out => {
                            let out = end == End::Out;
                            let at = *boundary.entry((e, out)).or_insert_with(|| {
                                sub.nodes.push(Node {
                                    size: Size::new(BOUNDARY, BOUNDARY),
                                    rank: if out {
                                        RankConstraint::Last
                                    } else {
                                        RankConstraint::First
                                    },
                                    ..Node::default()
                                });
                                sub.nodes.len() - 1
                            });
                            let side = if out { in_side } else { out_side };
                            // `t: 0` is the point the outer piece aims for.
                            (at, Some(PortHint::Offset { side, t: 0.0 }))
                        }
                    }
                };
                let (s, sh) = end(piece.from, true);
                let (t, th) = end(piece.to, false);
                let outermost = k == main_piece(list);
                edge.insert((e, k), sub.edges.len());
                sub.edges.push(Edge {
                    source: NodeId(s as u32),
                    target: NodeId(t as u32),
                    source_port: sh,
                    target_port: th,
                    label_size: if outermost {
                        g.edges[e].label_size
                    } else {
                        None
                    },
                    ..g.edges[e].clone()
                });
            }
        }
        let opts = if level.is_some() { &inner } else { o };
        let mut drawn = layout_flat(&sub, opts)?;
        let mut frame = Rect::default();
        if let Some(c) = level {
            snap_boundaries(&mut drawn, &sub, &boundary, flow_along_x);
            // The box excludes the boundary nodes.
            let pad = o.spacing.compound_padding;
            let real: Vec<usize> = members.iter().map(|m| index[m]).collect();
            let (mut lo, mut hi) = (
                (f64::INFINITY, f64::INFINITY),
                (f64::NEG_INFINITY, f64::NEG_INFINITY),
            );
            let mut grow = |p: Point| {
                lo = (lo.0.min(p.x), lo.1.min(p.y));
                hi = (hi.0.max(p.x), hi.1.max(p.y));
            };
            for &r in &real {
                let pl = drawn.nodes[r];
                grow(Point::new(
                    pl.center.x - pl.size.width / 2.0,
                    pl.center.y - pl.size.height / 2.0,
                ));
                grow(Point::new(
                    pl.center.x + pl.size.width / 2.0,
                    pl.center.y + pl.size.height / 2.0,
                ));
            }
            for route in &drawn.edges {
                for &p in &route.points {
                    grow(p);
                }
            }
            if !lo.0.is_finite() {
                lo = (0.0, 0.0);
                hi = (0.0, 0.0);
            }
            let own = g.nodes[c].size;
            let (cx, cy) = ((lo.0 + hi.0) / 2.0, (lo.1 + hi.1) / 2.0);
            let mut w = (hi.0 - lo.0 + 2.0 * pad).max(own.width);
            let mut h = (hi.1 - lo.1 + 2.0 * pad).max(own.height - header);
            // Lengthen the border so no outer port hint gets clamped away from its boundary node.
            for &b in boundary.values() {
                let at = drawn.nodes[b].center;
                if flow_along_x {
                    h = h.max(2.0 * (at.y - cy).abs() / PORT_REACH);
                } else {
                    w = w.max(2.0 * (at.x - cx).abs() / PORT_REACH);
                }
            }
            frame = Rect {
                min: Point::new(cx - w / 2.0, cy - h / 2.0),
                max: Point::new(cx + w / 2.0, cy + h / 2.0),
            };
            size[c] = Size::new(w, h);
            for (&(e, out), &b) in &boundary {
                let face = match (flow_along_x, out) {
                    (true, true) => frame.max.x,
                    (true, false) => frame.min.x,
                    (false, true) => frame.max.y,
                    (false, false) => frame.min.y,
                };
                let at = &mut drawn.nodes[b].center;
                if flow_along_x {
                    at.x = face;
                } else {
                    at.y = face;
                }
                let t = if flow_along_x {
                    (at.y - cy) / (h / 2.0)
                } else {
                    (at.x - cx) / (w / 2.0)
                };
                port_t.insert((c, e, out), t);
                for (_, &se) in edge.range((e, 0)..(e + 1, 0)) {
                    let points = &mut drawn.edges[se].points;
                    let end = if out && sub.edges[se].target.index() == b {
                        points.last_mut()
                    } else if !out && sub.edges[se].source.index() == b {
                        points.first_mut()
                    } else {
                        None
                    };
                    if let Some(p) = end {
                        if flow_along_x {
                            p.x = face;
                        } else {
                            p.y = face;
                        }
                    }
                }
            }
        }
        levels.insert(
            level,
            Level {
                layout: drawn,
                index,
                edge,
                frame,
            },
        );
    }

    // Outermost first, since each offset builds on its parent's.
    let mut offset: BTreeMap<Option<usize>, (f64, f64)> = BTreeMap::new();
    offset.insert(None, (0.0, 0.0));
    for &c in post.iter().rev() {
        let outer = parent[c];
        let (ox, oy) = offset[&outer];
        let at = levels[&outer].layout.nodes[levels[&outer].index[&c]].center;
        let f = levels[&Some(c)].frame;
        let (fx, fy) = ((f.min.x + f.max.x) / 2.0, (f.min.y + f.max.y) / 2.0);
        offset.insert(Some(c), (at.x + ox - fx, at.y + oy - fy));
    }
    let shift = |level: Option<usize>, p: Point| {
        let (dx, dy) = offset[&level];
        Point::new(p.x + dx, p.y + dy)
    };

    let nodes: Vec<Placement> = (0..n)
        .map(|i| {
            let lv = &levels[&parent[i]];
            let pl = lv.layout.nodes[lv.index[&i]];
            let mut center = shift(parent[i], pl.center);
            let mut size = size[i];
            if !children[i].is_empty() {
                center.y -= header / 2.0;
                size.height += header;
            }
            Placement { center, size, ..pl }
        })
        .collect();

    let mut edges = Vec::with_capacity(g.edges.len());
    let mut reversed = vec![];
    for (e, list) in pieces.iter().enumerate() {
        let mut points: Vec<Point> = vec![];
        let mut spans = 0u32;
        let mut kind = RouteKind::Direct;
        let mut label_anchor = None;
        for (k, piece) in list.iter().enumerate() {
            let lv = &levels[&piece.level];
            let Some(&se) = lv.edge.get(&(e, k)) else {
                continue;
            };
            let route = &lv.layout.edges[se];
            for (j, &p) in route.points.iter().enumerate() {
                let p = shift(piece.level, p);
                let joins = j == 0
                    && points.last().is_some_and(|q| {
                        (q.x - p.x).abs() < JOIN_EPS && (q.y - p.y).abs() < JOIN_EPS
                    });
                if !joins {
                    points.push(p);
                }
            }
            match route.kind {
                RouteKind::Chained { spans: s } => spans += s,
                RouteKind::SelfLoop | RouteKind::Bundled { .. } => kind = route.kind,
                _ => spans += 1,
            }
            if route.label_anchor.is_some() {
                label_anchor = route.label_anchor.map(|p| shift(piece.level, p));
            }
            if k == main_piece(list) && lv.layout.reversed.contains(&EdgeId(se as u32)) {
                reversed.push(EdgeId(e as u32));
            }
        }
        if kind == RouteKind::Direct && spans > 1 {
            kind = RouteKind::Chained { spans };
        }
        let points: Vec<Point> = simplify_collinear(points.iter().map(|p| (p.x, p.y)).collect())
            .into_iter()
            .map(|(x, y)| Point::new(x, y))
            .collect();
        let (s, t) = (g.edges[e].source.index(), g.edges[e].target.index());
        edges.push(EdgeRoute::routed(
            points,
            kind,
            &nodes[s],
            &nodes[t],
            label_anchor,
        ));
    }

    let mut junctions = vec![];
    let mut lanes = vec![];
    let mut dummy_count = 0;
    for (&level, lv) in &levels {
        junctions.extend(lv.layout.junctions.iter().map(|&p| shift(level, p)));
        dummy_count += lv.layout.stats.dummy_count;
        if level.is_none() {
            lanes.extend(lv.layout.lanes.iter().map(|l| Lane {
                rect: Rect {
                    min: shift(level, l.rect.min),
                    max: shift(level, l.rect.max),
                },
                ..*l
            }));
        }
    }
    let mut out = Layout {
        nodes,
        edges,
        bounds: Rect::default(),
        layer_count: levels[&None].layout.layer_count,
        reversed,
        stats: LayoutStats { dummy_count },
        junctions,
        lanes,
    };
    out.recompute_bounds();
    Ok(out)
}
