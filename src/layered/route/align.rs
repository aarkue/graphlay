//! Local port fixes after spreading, run in this order: uncross parallel pairs, slide multi-hop
//! ports onto their channel, snap facing ports of direct edges onto one line, relax ports crowded
//! across a corner, and pair a direct edge's ports straight across the gutter.
//!
//! Every fix slides a port along its own border, so side hints survive. Offset-pinned ports never
//! move.

use crate::geometry::fp;
use std::collections::BTreeMap;

use super::{ArcEnds, PortMap, RouteCtx, PORT_SPAN};
use crate::geometry::hit::segments_cross;
use crate::geometry::Side;
use crate::layered::ports::project_port;
use crate::sort::SortDyn;

/// Largest offset between facing ports that [`align_facing`] treats as a dogleg to remove.
const FACING_ALIGN_MAX: f64 = 24.0;
/// Smallest drop in crowding potential that justifies a relaxation move.
const RELAX_MIN_GAIN: f64 = 1e-3;

/// Every arc end as `((arc, is_source), node, side index)`.
fn end_borders(arc_ends: &ArcEnds) -> Vec<((usize, bool), usize, u8)> {
    arc_ends
        .iter()
        .flat_map(|(&a, (s, t))| {
            [
                ((a, true), s.node, s.side.index()),
                ((a, false), t.node, t.side.index()),
            ]
        })
        .collect()
}

/// Swaps ports at a shared endpoint when two edges between the same node pair cross.
pub(super) fn uncross_bundles(ctx: &RouteCtx, port: &mut PortMap) {
    let chains = ctx.chains;
    let ends = |arc: usize| -> (usize, usize) {
        let c = &chains[&arc];
        (c[0], c[c.len() - 1])
    };
    let mut bundles: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    for (&a, c) in chains.iter().filter(|(_, c)| c.len() >= 2) {
        let (lo, hi) = (c[0], c[c.len() - 1]);
        bundles.entry((lo.min(hi), lo.max(hi))).or_default().push(a);
    }
    let seg_x = |p1: ((f64, f64), (f64, f64)), p2: ((f64, f64), (f64, f64))| -> bool {
        segments_cross(p1.0, p1.1, p2.0, p2.1)
    };
    for ((_, _), arcs) in bundles.iter().filter(|(_, v)| v.len() >= 2) {
        let mut sorted = arcs.clone();
        sorted.sort_unstable_by_dyn(Ord::cmp);
        for i in 0..sorted.len() {
            for j in i + 1..sorted.len() {
                let (a, b) = (sorted[i], sorted[j]);
                let (a_lo, a_hi) = ends(a);
                let (b_lo, b_hi) = ends(b);
                let seg = |arc: usize| -> ((f64, f64), (f64, f64)) {
                    (port[&(arc, true)].anchor, port[&(arc, false)].anchor)
                };
                if !seg_x(seg(a), seg(b)) {
                    continue;
                }
                // All four pairings matter so an anti-parallel 2-cycle also gets parallel lanes.
                let (ka, kb) = if a_hi == b_hi {
                    ((a, false), (b, false))
                } else if a_lo == b_lo {
                    ((a, true), (b, true))
                } else if a_lo == b_hi {
                    ((a, true), (b, false))
                } else if a_hi == b_lo {
                    ((a, false), (b, true))
                } else {
                    continue;
                };
                let pinned = |k: (usize, bool)| ctx.hints.pinned(k.0, k.1);
                if port[&ka].side.index() == port[&kb].side.index() && !pinned(ka) && !pinned(kb) {
                    let (pa, pb) = (port[&ka], port[&kb]);
                    port.insert(ka, pb);
                    port.insert(kb, pa);
                }
            }
        }
    }
}

/// Slides a multi-hop edge's end port onto its dummy channel's order coordinate when the spot is on
/// the border, unobstructed and keeps the port gap.
pub(super) fn align_channels(ctx: &RouteCtx, arc_ends: &ArcEnds, port: &mut PortMap) {
    let (b, stub_len, min_port_gap) = (ctx.b, ctx.stub_len, ctx.min_port_gap);
    let (cy, ow) = (&b.cy, &b.order_w);
    let on_side = end_borders(arc_ends);
    for (arc, chain) in ctx.ordered() {
        let m = chain.len();
        if m <= 2 {
            continue;
        }
        for (is_source, node, adj) in [
            (true, chain[0], chain[1]),
            (false, chain[m - 1], chain[m - 2]),
        ] {
            let p = port[&(arc, is_source)];
            if !p.side.horizontal() || ctx.radial(node) || ctx.hints.pinned(arc, is_source) {
                continue;
            }
            let (track, old) = (cy[adj], p.anchor.1);
            if (track - old).abs() < 0.5 || (track - cy[node]).abs() > ow[node] * PORT_SPAN {
                continue;
            }
            let ok = on_side.iter().all(|&(key, n2, s2)| {
                if key == (arc, is_source) || n2 != node || s2 != p.side.index() {
                    return true;
                }
                let other = port[&key].anchor.1;
                (other - track).abs() >= min_port_gap
                    && ((other < old.min(track)) || (other > old.max(track)))
            });
            if ok {
                port.insert(
                    (arc, is_source),
                    project_port(node, p.side, track - cy[node], b, stub_len),
                );
            }
        }
    }
}

/// Snaps the facing ports of a direct edge onto one order coordinate when they are less than
/// [`FACING_ALIGN_MAX`] apart. Radial fans keep their spread.
pub(super) fn align_facing(ctx: &RouteCtx, arc_ends: &ArcEnds, port: &mut PortMap) {
    let (b, stub_len, min_port_gap) = (ctx.b, ctx.stub_len, ctx.min_port_gap);
    let (cy, ow) = (&b.cy, &b.order_w);
    let on_side = end_borders(arc_ends);
    for (arc, chain) in ctx.ordered() {
        if chain.len() != 2 {
            continue;
        }
        let (s, t) = (port[&(arc, true)], port[&(arc, false)]);
        if !(s.side == Side::Right && t.side == Side::Left) {
            continue;
        }
        let (ys, yt) = (s.anchor.1, t.anchor.1);
        if (ys - yt).abs() < 0.5 || (ys - yt).abs() > FACING_ALIGN_MAX {
            continue;
        }
        let feasible = |node: usize, key: (usize, bool), side: Side, tgt: f64, old: f64| -> bool {
            if ctx.radial(node)
                || ctx.hints.pinned(key.0, key.1)
                || (tgt - cy[node]).abs() > ow[node] * PORT_SPAN
            {
                return false;
            }
            on_side.iter().all(|&(k2, n2, s2)| {
                if k2 == key || n2 != node || s2 != side.index() {
                    return true;
                }
                let other = port[&k2].anchor.1;
                let gap = min_port_gap
                    .max((ctx.thickness(arc) + ctx.thickness(k2.0)) / 2.0 + ctx.port_stroke_clear);
                (other - tgt).abs() >= gap && ((other < old.min(tgt)) || (other > old.max(tgt)))
            })
        };
        let (s_node, t_node) = (chain[0], chain[1]);
        if feasible(t_node, (arc, false), t.side, ys, yt) {
            port.insert(
                (arc, false),
                project_port(t_node, t.side, ys - cy[t_node], b, stub_len),
            );
        } else if feasible(s_node, (arc, true), s.side, yt, ys) {
            port.insert(
                (arc, true),
                project_port(s_node, s.side, yt - cy[s_node], b, stub_len),
            );
        }
    }
}

/// Ports on adjacent sides can nearly touch at a corner. Per node, greedily moves ports apart while
/// the crowding drops. Channel-aligned ports stay.
pub(super) fn relax_cross_side(ctx: &RouteCtx, arc_ends: &ArcEnds, port: &mut PortMap) {
    let (b, o, stub_len, chains) = (ctx.b, ctx.o, ctx.stub_len, ctx.chains);
    let (cx, cy, lw, ow) = (&b.cx, &b.cy, &b.layer_w, &b.order_w);
    let (sep_min, step) = (
        o.tuning.routing.port_relax_clear,
        o.tuning.routing.port_relax_step,
    );
    type NodePorts = Vec<((usize, bool), Side)>;
    let mut by_node: BTreeMap<usize, NodePorts> = BTreeMap::new();
    for &arc in &ctx.arc_keys {
        let Some((s, t)) = arc_ends.get(&arc) else {
            continue;
        };
        by_node
            .entry(s.node)
            .or_default()
            .push(((arc, true), s.side));
        by_node
            .entry(t.node)
            .or_default()
            .push(((arc, false), t.side));
    }
    let mut node_ids: Vec<usize> = by_node.keys().copied().collect();
    node_ids.sort_unstable_by_dyn(Ord::cmp);
    for node in node_ids {
        if ctx.radial(node) {
            continue; // a radial fan already keeps an angular gap
        }
        let entries = &by_node[&node];
        if entries.len() < 2 {
            continue;
        }
        let pinned: Vec<bool> = entries
            .iter()
            .map(|&((arc, is_source), side)| {
                if ctx.hints.pinned(arc, is_source) {
                    return true;
                }
                if !side.horizontal() {
                    return false;
                }
                let chain = &chains[&arc];
                if chain.len() <= 2 {
                    return false;
                }
                let adj = if is_source {
                    chain[1]
                } else {
                    chain[chain.len() - 2]
                };
                (cy[adj] - port[&(arc, is_source)].anchor.1).abs() < 0.75
            })
            .collect();
        let dist = |a: (f64, f64), c: (f64, f64)| fp::hypot(a.0 - c.0, a.1 - c.1);
        // Squared shortfalls summed over all pairs, so every bad pair pulls on the descent.
        let potential = |anch: &[(f64, f64)]| -> f64 {
            let mut p = 0.0;
            for i in 0..anch.len() {
                for j in i + 1..anch.len() {
                    let short = (sep_min - dist(anch[i], anch[j])).max(0.0);
                    p += short * short;
                }
            }
            p
        };
        for _ in 0..o.tuning.routing.port_relax_rounds {
            let anchors: Vec<(f64, f64)> = entries.iter().map(|(k, _)| port[k].anchor).collect();
            let p0 = potential(&anchors);
            if p0 <= 1e-6 {
                break;
            }
            let mut best: Option<(usize, f64, f64)> = None; // (entry, new off, new potential)
            for (e, &(_, side)) in entries.iter().enumerate() {
                if pinned[e] {
                    continue;
                }
                let horiz = side.horizontal();
                let cur = if horiz {
                    anchors[e].1 - cy[node]
                } else {
                    anchors[e].0 - cx[node]
                };
                let lim = (if horiz { ow[node] } else { lw[node] }) * PORT_SPAN;
                // The potential is not unimodal along a border, so long jumps let a port pass a
                // blocking neighbour.
                for dir in [
                    -5.0 * step,
                    -3.0 * step,
                    -step,
                    step,
                    3.0 * step,
                    5.0 * step,
                ] {
                    let o2 = (cur + dir).clamp(-lim, lim);
                    if (o2 - cur).abs() < 0.5 {
                        continue;
                    }
                    let mut trial = anchors.clone();
                    trial[e] = project_port(node, side, o2, b, stub_len).anchor;
                    let p1 = potential(&trial);
                    if p1 < p0 - RELAX_MIN_GAIN && best.is_none_or(|(_, _, bp)| p1 < bp) {
                        best = Some((e, o2, p1));
                    }
                }
            }
            match best {
                Some((e, o2, _)) => {
                    let (key, side) = entries[e];
                    port.insert(key, project_port(node, side, o2, b, stub_len));
                }
                None => break,
            }
        }
    }
}

/// Straightens a direct edge whose facing ports sit on different order lines by sliding one port
/// onto the other's line, if that stays on the border and clears every box.
pub(super) fn pair_direct(ctx: &RouteCtx, arc_ends: &ArcEnds, port: &mut PortMap) {
    let (b, o, stub_len, min_port_gap) = (ctx.b, ctx.o, ctx.stub_len, ctx.min_port_gap);
    let (cx, cy, ow) = (&b.cx, &b.cy, &b.order_w);
    let all_ports = end_borders(arc_ends);
    for (arc, chain) in ctx.ordered() {
        if chain.len() != 2 {
            continue;
        }
        let (lo, hi) = (chain[0], chain[1]);
        let (ps, pt) = (port[&(arc, true)], port[&(arc, false)]);
        if !ps.side.horizontal() || !pt.side.horizontal() {
            continue;
        }
        if (ps.anchor.1 - pt.anchor.1).abs() < 0.5 {
            continue;
        }
        for (is_source, node, y2) in [(true, lo, pt.anchor.1), (false, hi, ps.anchor.1)] {
            let p = port[&(arc, is_source)];
            let old = p.anchor.1;
            if ctx.radial(node)
                || ctx.hints.pinned(arc, is_source)
                || (y2 - cy[node]).abs() > ow[node] * PORT_SPAN
            {
                continue;
            }
            // A lone port leaves the centre of its border only for a jog that is clearly visible.
            let lone = all_ports
                .iter()
                .filter(|&&(_, n2, s2)| n2 == node && s2 == p.side.index())
                .count()
                == 1;
            if lone && (y2 - old).abs() <= ow[node] * 0.3 {
                continue;
            }
            if ctx.seg_hits((cx[lo], y2), (cx[hi], y2), lo, hi) {
                continue;
            }
            let trial = project_port(node, p.side, y2 - cy[node], b, stub_len);
            let ok = all_ports.iter().all(|&(key, n2, s2)| {
                if key == (arc, is_source) || n2 != node {
                    return true;
                }
                let q = port[&key];
                if s2 == p.side.index() {
                    (q.anchor.1 - y2).abs() >= min_port_gap
                        && ((q.anchor.1 < old.min(y2)) || (q.anchor.1 > old.max(y2)))
                } else {
                    fp::hypot(q.anchor.0 - trial.anchor.0, q.anchor.1 - trial.anchor.1)
                        >= o.tuning.routing.port_relax_clear
                }
            });
            if ok {
                port.insert((arc, is_source), trial);
                break;
            }
        }
    }
}
