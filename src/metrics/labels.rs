//! Label collisions.
//!
//! Checks whether each reserved label box ([`crate::layout::EdgeRoute::label_anchor`]) lands on
//! another label, a node or another edge's polyline. A label on a foreign edge makes it hard to
//! tell which edge the label belongs to (Burattin et al.). A label's own edge is never counted
//! against it, since a mid-edge label always sits on it.

use super::{
    box_gap, box_penetration, segment_box_distance, segment_hits_node, EdgeGeom, LabelBox,
    LayoutMetrics, NodeBox, Problem, LABEL_OVERLAP_TOL,
};

/// Fills [`LayoutMetrics::label_count`], the three collision counts, their total and
/// [`LayoutMetrics::min_label_clearance`].
pub(super) fn fill_label_metrics(
    problem: &Problem,
    boxes: &[NodeBox],
    edges: &[EdgeGeom],
    m: &mut LayoutMetrics,
) {
    let labels = &problem.labels;
    m.label_count = labels.len();
    if labels.is_empty() {
        return;
    }
    let rects: Vec<NodeBox> = labels.iter().map(LabelBox::as_box).collect();
    let mut clearance: Option<f64> = None;
    let mut note = |d: f64| {
        if d.is_finite() {
            clearance = Some(clearance.map_or(d, |c: f64| c.min(d)));
        }
    };

    for (i, a) in rects.iter().enumerate() {
        for b in rects.iter().skip(i + 1) {
            let (ox, oy) = box_penetration(a, b);
            if ox > LABEL_OVERLAP_TOL && oy > LABEL_OVERLAP_TOL {
                m.label_label_overlaps += 1;
                note(0.0);
            } else {
                note(box_gap(a, b));
            }
        }
    }

    // Every node counts, including the label's own source and target.
    for a in &rects {
        for b in boxes {
            let (ox, oy) = box_penetration(a, b);
            if ox > LABEL_OVERLAP_TOL && oy > LABEL_OVERLAP_TOL {
                m.label_node_overlaps += 1;
                note(0.0);
            } else {
                note(box_gap(a, b));
            }
        }
    }

    // Once per (label, foreign edge) pair.
    for (label, rect) in labels.iter().zip(rects.iter()) {
        for e in edges {
            if e.id == label.edge {
                continue;
            }
            let mut hit = false;
            let mut nearest = f64::INFINITY;
            for s in e.pts.windows(2) {
                if segment_hits_node(s[0], s[1], rect) {
                    hit = true;
                    break;
                }
                nearest = nearest.min(segment_box_distance(s[0], s[1], rect));
            }
            if hit {
                m.label_edge_hits += 1;
                note(0.0);
            } else {
                note(nearest);
            }
        }
    }

    m.label_overlaps = m.label_label_overlaps + m.label_node_overlaps + m.label_edge_hits;
    m.min_label_clearance = clearance;
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::super::measure;
    use crate::geometry::{Point, Size};
    use crate::layout::{EdgeRoute, Layout, Placement};
    use crate::{Edge, Graph, LayoutOptions, Node, NodeId};

    const NODE: Size = Size {
        width: 60.0,
        height: 30.0,
    };

    fn node() -> Node {
        Node {
            size: NODE,
            ..Node::default()
        }
    }

    fn place(x: f64, y: f64) -> Placement {
        Placement {
            center: Point::new(x, y),
            size: NODE,
            ..Placement::default()
        }
    }

    fn route(pts: &[[f64; 2]], anchor: Option<[f64; 2]>) -> EdgeRoute {
        EdgeRoute {
            points: pts.iter().map(|p| Point::new(p[0], p[1])).collect(),
            label_anchor: anchor.map(|a| Point::new(a[0], a[1])),
            ..EdgeRoute::default()
        }
    }

    fn labelled(source: u32, target: u32) -> Edge {
        Edge {
            label_size: Some(Size::new(40.0, 16.0)),
            ..Edge::new(NodeId(source), NodeId(target))
        }
    }

    #[test]
    fn two_labels_on_the_same_spot_collide() {
        let graph = Graph {
            nodes: vec![node(), node(), node(), node()],
            edges: vec![labelled(0, 1), labelled(2, 3)],
        };
        let nodes = vec![
            place(0.0, 0.0),
            place(0.0, 400.0),
            place(300.0, 0.0),
            place(300.0, 400.0),
        ];
        let clash = Layout {
            nodes: nodes.clone(),
            edges: vec![
                route(&[[0.0, 15.0], [0.0, 385.0]], Some([150.0, 200.0])),
                route(&[[300.0, 15.0], [300.0, 385.0]], Some([155.0, 200.0])),
            ],
            ..Layout::default()
        };
        let m = measure(&graph, &LayoutOptions::default(), &clash);
        assert_eq!(m.label_count, 2);
        assert_eq!(m.label_label_overlaps, 1);
        assert_eq!(m.label_overlaps, 1);
        assert_eq!(m.min_label_clearance, Some(0.0));

        let apart = Layout {
            nodes,
            edges: vec![
                route(&[[0.0, 15.0], [0.0, 385.0]], Some([100.0, 200.0])),
                route(&[[300.0, 15.0], [300.0, 385.0]], Some([200.0, 200.0])),
            ],
            ..Layout::default()
        };
        let m = measure(&graph, &LayoutOptions::default(), &apart);
        assert_eq!(m.label_label_overlaps, 0);
        assert_eq!(m.label_overlaps, 0);
        // 100px between centres minus 40px of label width.
        assert_eq!(m.min_label_clearance, Some(60.0));
    }

    #[test]
    fn a_label_over_a_node_is_counted() {
        let graph = Graph {
            nodes: vec![node(), node(), node()],
            edges: vec![labelled(0, 1)],
        };
        let l = Layout {
            nodes: vec![place(0.0, 0.0), place(0.0, 400.0), place(0.0, 200.0)],
            edges: vec![route(&[[0.0, 15.0], [0.0, 385.0]], Some([0.0, 200.0]))],
            ..Layout::default()
        };
        let m = measure(&graph, &LayoutOptions::default(), &l);
        assert_eq!(m.label_node_overlaps, 1);
        assert_eq!(m.label_overlaps, 1);
    }

    #[test]
    fn only_a_foreign_edge_counts_against_a_label() {
        let graph = Graph {
            nodes: vec![node(), node(), node(), node()],
            edges: vec![labelled(0, 1), Edge::new(NodeId(2), NodeId(3))],
        };
        let nodes = vec![
            place(0.0, 0.0),
            place(0.0, 400.0),
            place(-300.0, 200.0),
            place(300.0, 200.0),
        ];
        // The second edge runs straight through the label at (0, 200).
        let through = Layout {
            nodes: nodes.clone(),
            edges: vec![
                route(&[[0.0, 15.0], [0.0, 385.0]], Some([0.0, 200.0])),
                route(&[[-270.0, 200.0], [270.0, 200.0]], None),
            ],
            ..Layout::default()
        };
        let m = measure(&graph, &LayoutOptions::default(), &through);
        assert_eq!(m.label_edge_hits, 1, "{m:?}");
        assert_eq!(m.label_label_overlaps, 0);

        let clear = Layout {
            nodes,
            edges: vec![
                route(&[[0.0, 15.0], [0.0, 385.0]], Some([0.0, 200.0])),
                route(&[[-270.0, 340.0], [270.0, 340.0]], None),
            ],
            ..Layout::default()
        };
        assert_eq!(
            measure(&graph, &LayoutOptions::default(), &clear).label_edge_hits,
            0
        );
    }

    /// No labels gives `None` clearance, not a perfect score.
    #[test]
    fn an_unlabelled_drawing_reports_no_labels() {
        let graph = Graph {
            nodes: vec![node(), node()],
            edges: vec![Edge::new(NodeId(0), NodeId(1))],
        };
        let l = Layout {
            nodes: vec![place(0.0, 0.0), place(0.0, 400.0)],
            edges: vec![route(&[[0.0, 15.0], [0.0, 385.0]], None)],
            ..Layout::default()
        };
        let m = measure(&graph, &LayoutOptions::default(), &l);
        assert_eq!(m.label_count, 0);
        assert_eq!(m.label_overlaps, 0);
        assert_eq!(m.min_label_clearance, None);
    }

    /// A label without an anchor has no box and is not counted.
    #[test]
    fn a_label_without_an_anchor_is_not_a_label() {
        let graph = Graph {
            nodes: vec![node(), node()],
            edges: vec![labelled(0, 1)],
        };
        let l = Layout {
            nodes: vec![place(0.0, 0.0), place(0.0, 400.0)],
            edges: vec![route(&[[0.0, 15.0], [0.0, 385.0]], None)],
            ..Layout::default()
        };
        assert_eq!(
            measure(&graph, &LayoutOptions::default(), &l).label_count,
            0
        );
    }
}
