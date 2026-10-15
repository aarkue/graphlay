//! A plain drawing of a graph and its layout, for a quick look.
//!
//! Nodes are boxes and ellipses labelled with their keys, edges are polylines with arrowheads,
//! compounds are dashed frames, and lanes sit behind everything.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::graph::{Graph, NodeId};
use crate::layout::Layout;
use crate::svg::model::{
    Attrs, BuiltinMarker, MarkerRef, NodeContent, Overlay, Point, RawSvg, Rect, Scene, SceneEdge,
    SceneNode, Shape, Size, Stroke, TextRun, TextWrap, ZOrder,
};
use crate::svg::render::{render, xml_escape};
use crate::svg::text::HeuristicMeasurer;
use crate::svg::theme::Theme;

/// What a sketch shows beyond the geometry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct SketchOptions {
    /// A label per node. Empty labels every node with its key.
    pub labels: Vec<String>,
    /// Lane names by group id, for a drawing with swimlanes.
    pub lane_names: Vec<String>,
    /// The page background. `None` leaves it transparent.
    pub background: Option<String>,
    /// Space around the drawing, in px.
    pub padding: f64,
}

impl Default for SketchOptions {
    fn default() -> Self {
        SketchOptions {
            labels: Vec::new(),
            lane_names: Vec::new(),
            background: Some("#ffffff".to_string()),
            padding: 24.0,
        }
    }
}

/// Edge colours by group, for drawings whose edges carry a [`crate::GroupId`].
const EDGE_COLOURS: &[&str] = &[
    "#2563eb", "#dc2626", "#059669", "#d97706", "#7c3aed", "#db2777",
];

const JUNCTION_R: f64 = 3.0;

/// Draws `layout` of `graph` to SVG markup with the default theme.
pub fn sketch(graph: &Graph, layout: &Layout, options: &SketchOptions) -> String {
    render(
        &sketch_scene(graph, layout, options),
        &Theme::default(),
        &HeuristicMeasurer::new(),
    )
}

/// The sketch as a [`Scene`], to restyle before rendering. Scene node `i` is graph node `i`
/// unless the graph has compounds, which come first so their children are drawn over them.
pub fn sketch_scene(graph: &Graph, layout: &Layout, options: &SketchOptions) -> Scene {
    let g = graph;
    let l = layout;
    let is_compound = |i: usize| g.nodes.iter().any(|n| n.parent == Some(NodeId(i as u32)));
    let label = |i: usize| -> String {
        if options.labels.is_empty() {
            g.nodes[i].key.clone().unwrap_or_default()
        } else {
            options.labels.get(i).cloned().unwrap_or_default()
        }
    };
    let nodes = node_order(graph, layout)
        .into_iter()
        .map(|i| {
            let at = &l.nodes[i];
            let compound = is_compound(i);
            let shape = if matches!(g.nodes[i].shape, crate::Shape::Ellipse) {
                Shape::Ellipse
            } else {
                Shape::Rect { corner_radius: 5.0 }
            };
            let text = label(i);
            let content = if text.is_empty() {
                Vec::new()
            } else if compound {
                vec![NodeContent::Text(TextRun {
                    color: Some("#5b6577".into()),
                    ..TextRun::new(text)
                        .at(0.0, -at.size.height / 2.0 + 13.0)
                        .size(11.0)
                        .weight(600.0)
                })]
            } else {
                vec![NodeContent::Text(TextRun {
                    color: Some("#1d2533".into()),
                    wrap: Some(TextWrap {
                        max_width: at.size.width - 12.0,
                        max_lines: 2,
                        line_height: 13.5,
                    }),
                    ..TextRun::new(text).size(12.0).weight(500.0)
                })]
            };
            let stroke = if compound {
                Stroke::new("#8a94a6", 1.25).dashed("5 4")
            } else {
                Stroke::new("#2f3a4d", 1.6)
            };
            SceneNode {
                rect: Rect::from_center(
                    Point::new(at.center.x, at.center.y),
                    Size::new(at.size.width, at.size.height),
                ),
                shape,
                fill: Some(if compound { "none" } else { "#ffffff" }.into()),
                stroke: Some(stroke),
                content,
                attrs: Attrs::default(),
            }
        })
        .collect();
    let edges = l
        .edges
        .iter()
        .zip(&g.edges)
        .filter(|(route, _)| route.points.len() >= 2)
        .map(|(route, edge)| {
            let colour = edge.group.map_or("#3b4658", |gid| {
                EDGE_COLOURS[gid.0 as usize % EDGE_COLOURS.len()]
            });
            SceneEdge {
                points: route.points.iter().map(|p| Point::new(p.x, p.y)).collect(),
                stroke: Stroke::new(colour, edge.thickness.clamp(1.4, 6.0)),
                corner_radius: 5.0,
                hop_radius: 4.0,
                // Keeps sub-pixel kinks from becoming rounded corners.
                simplify: 3.0,
                marker_end: MarkerRef::builtin(BuiltinMarker::Arrow),
                ..SceneEdge::default()
            }
        })
        .collect();

    let mut overlays = Vec::new();
    if !l.junctions.is_empty() {
        overlays.push(Overlay::raw(junction_dots(l)));
    }
    if !l.lanes.is_empty() {
        overlays.push(Overlay {
            z: ZOrder::Under,
            ..Overlay::raw(lane_bands(l, &options.lane_names))
        });
    }
    Scene {
        background: options.background.clone(),
        padding: options.padding,
        nodes,
        edges,
        overlays,
        ..Scene::default()
    }
}

/// Graph node indices in paint order, outermost compounds first.
fn node_order(graph: &Graph, layout: &Layout) -> Vec<usize> {
    let depth = |mut i: usize| {
        let mut d = 0;
        while let Some(parent) = graph.nodes[i].parent {
            d += 1;
            i = parent.index();
        }
        d
    };
    let mut order: Vec<usize> = (0..graph.nodes.len().min(layout.nodes.len())).collect();
    order.sort_by_key(|&i| (depth(i), i));
    order
}

/// Alternating lane fills, a hairline between lanes, and each lane's name.
fn lane_bands(layout: &Layout, names: &[String]) -> RawSvg {
    let mut markup = String::new();
    let mut bounds: Option<Rect> = None;
    for (k, lane) in layout.lanes.iter().enumerate() {
        let r = Rect::new(
            lane.rect.min.x,
            lane.rect.min.y,
            lane.rect.max.x - lane.rect.min.x,
            lane.rect.max.y - lane.rect.min.y,
        );
        let fill = if k % 2 == 0 { "#f3f5f8" } else { "#fafbfc" };
        let name = lane
            .group
            .and_then(|gid| names.get(gid.0 as usize))
            .map_or("Other", String::as_str);
        let _ = write!(
            markup,
            r##"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" fill="{fill}" stroke="#d5dae1" stroke-width="1"/><text x="{:.2}" y="{:.2}" font-size="11" font-weight="600" fill="#6b7280" font-family="sans-serif">{}</text>"##,
            r.x,
            r.y,
            r.w,
            r.h,
            r.x + 6.0,
            r.y + 14.0,
            xml_escape(name)
        );
        bounds = Some(bounds.map_or(r, |b| b.union(&r)));
    }
    RawSvg::new(markup, bounds.unwrap_or_default())
}

/// A dot at each branch point of a merged edge.
fn junction_dots(layout: &Layout) -> RawSvg {
    let mut markup = String::new();
    let mut bounds: Option<Rect> = None;
    for p in &layout.junctions {
        let _ = write!(
            markup,
            r##"<circle cx="{:.2}" cy="{:.2}" r="{JUNCTION_R}" fill="#3b4658"/>"##,
            p.x, p.y
        );
        let dot = Rect::new(
            p.x - JUNCTION_R,
            p.y - JUNCTION_R,
            2.0 * JUNCTION_R,
            2.0 * JUNCTION_R,
        );
        bounds = Some(bounds.map_or(dot, |b| b.union(&dot)));
    }
    RawSvg::new(markup, bounds.unwrap_or_default())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{GraphBuilder, LayoutOptions, Size};

    #[test]
    fn a_sketch_labels_nodes_with_their_keys_unless_told_otherwise() {
        let mut b = GraphBuilder::new();
        let a = b.add_box(Size::new(80.0, 40.0));
        let c = b.add_box(Size::new(80.0, 40.0));
        b.connect(a, c);
        let mut g = b.build();
        g.nodes[0].key = Some("start".into());
        let l = crate::layout(&g, &LayoutOptions::layered_default()).unwrap();

        let plain = sketch(&g, &l, &SketchOptions::default());
        assert!(plain.contains("start"));
        let named = sketch(
            &g,
            &l,
            &SketchOptions {
                labels: vec!["Alpha".into(), "Beta".into()],
                ..Default::default()
            },
        );
        assert!(named.contains("Alpha") && named.contains("Beta") && !named.contains("start"));
        assert_eq!(
            sketch_scene(&g, &l, &SketchOptions::default()).edges.len(),
            1
        );
    }
}
