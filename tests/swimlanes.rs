//! Strict swimlanes ([`GroupLanes::Strict`]) and [`graphlay::Node::lane_key`].

use std::path::PathBuf;

use graphlay::fixture;
use graphlay::{
    layout, Edge, GraphBuilder, GroupId, GroupLanes, Layout, LayoutOptions, Node, NodeId, Size,
};

const EPS: f64 = 0.5;

/// The lane holding `group`, as `(lo, hi)` on the order axis (`y` in a left-to-right drawing).
fn lane_of(l: &Layout, group: Option<GroupId>, across_y: bool) -> (f64, f64) {
    let lane = l
        .lanes
        .iter()
        .find(|lane| lane.group == group)
        .unwrap_or_else(|| panic!("no lane for {group:?}"));
    if across_y {
        (lane.rect.min.y, lane.rect.max.y)
    } else {
        (lane.rect.min.x, lane.rect.max.x)
    }
}

/// Every grouped fixture, laid out in strict lanes: the lanes tile the order axis without
/// overlapping, and every node's box lies inside its own group's lane.
#[test]
fn every_node_sits_inside_its_own_lane() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut checked = 0;
    for f in fixture::load_dir(&dir).expect("fixtures load") {
        if f.options.tree || f.nodes.len() > 400 {
            continue;
        }
        let (graph, mut o) = f.to_graph();
        let mut groups: Vec<u32> = graph
            .nodes
            .iter()
            .filter_map(|n| n.group.map(|g| g.0))
            .collect();
        groups.sort_unstable();
        groups.dedup();
        if groups.len() < 2 {
            continue;
        }
        o.order.group_lanes = GroupLanes::Strict;
        let l = layout(&graph, &o).unwrap_or_else(|e| panic!("{}: {e}", f.name));
        assert!(!l.lanes.is_empty(), "{}: no lanes", f.name);
        let across_y = matches!(o.direction, graphlay::Direction::LeftRight);
        // Disjoint and in order.
        for w in l.lanes.windows(2) {
            let (a, b) = if across_y {
                (w[0].rect.max.y, w[1].rect.min.y)
            } else {
                (w[0].rect.max.x, w[1].rect.min.x)
            };
            assert!(a <= b + EPS, "{}: lanes overlap ({a} > {b})", f.name);
        }
        for (i, (n, p)) in graph.nodes.iter().zip(&l.nodes).enumerate() {
            let (lo, hi) = lane_of(&l, n.group, across_y);
            let (c, half) = if across_y {
                (p.center.y, n.size.height / 2.0)
            } else {
                (p.center.x, n.size.width / 2.0)
            };
            assert!(
                c - half >= lo - EPS && c + half <= hi + EPS,
                "{}: node {i} ({:?}) at {}..{} outside its lane {lo}..{hi}",
                f.name,
                n.group,
                c - half,
                c + half
            );
        }
        checked += 1;
    }
    assert!(checked >= 3, "only {checked} grouped fixtures checked");
}

/// One source fanning out to three nodes of one group in the next layer: their order follows
/// their lane keys, not the crossing minimiser's preference.
#[test]
fn lane_keys_order_a_lane() {
    let mut b = GraphBuilder::new();
    let node = |group: Option<u32>, key: Option<i64>| Node {
        size: Size::new(60.0, 30.0),
        group: group.map(GroupId),
        lane_key: key,
        ..Node::default()
    };
    b.add_node(node(Some(0), None));
    b.add_node(node(Some(1), Some(3)));
    b.add_node(node(Some(1), Some(1)));
    b.add_node(node(Some(1), Some(2)));
    for t in 1..4 {
        b.add_edge(Edge::new(NodeId(0), NodeId(t)));
    }
    let g = b.build();
    let mut o = LayoutOptions::default();
    o.order.group_lanes = GroupLanes::Strict;
    let l = layout(&g, &o).expect("lays out");
    let y = |i: usize| l.nodes[i].center.y;
    assert!(y(2) < y(3) && y(3) < y(1), "{} {} {}", y(2), y(3), y(1));
    // Two lanes, group 0 and group 1, and no "ungrouped" lane: every node has a group.
    let groups: Vec<_> = l.lanes.iter().map(|lane| lane.group).collect();
    assert_eq!(groups.len(), 2);
    assert!(groups.contains(&Some(GroupId(0))) && groups.contains(&Some(GroupId(1))));
}

/// Soft lanes, the default, report no rectangles.
#[test]
fn soft_lanes_report_nothing() {
    let mut b = GraphBuilder::new();
    for g in [0, 1] {
        b.add_node(Node {
            size: Size::new(60.0, 30.0),
            group: Some(GroupId(g)),
            ..Node::default()
        });
    }
    b.add_edge(Edge::new(NodeId(0), NodeId(1)));
    let l = layout(&b.build(), &LayoutOptions::default()).expect("lays out");
    assert!(l.lanes.is_empty());
}
