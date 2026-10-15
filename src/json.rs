//! JSON requests and string-in, string-out entry points.
//!
//! Every field of [`crate::Graph`] and [`crate::LayoutOptions`] is optional, enums are lowercase
//! or snake_case strings, and the options reject unknown fields. A request names a [`Preset`] and
//! patches it:
//!
//! ```json
//! { "graph": { "nodes": [ ... ], "edges": [ ... ] },
//!   "preset": "process_map",
//!   "options": { "routing": { "style": "diagonal" }, "wrap": { "viewport_aspect": 1.78 } } }
//! ```
//!
//! The patch is deep-merged over the preset, and an invalid patch is an error naming the field.
//! Available with the `serde` cargo feature, on by default.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::graph::Graph;
use crate::layout::Layout;
use crate::options::{LayoutOptions, Preset};
use crate::tree::TreeOptions;

/// A layered-layout request: the graph, the preset its options start from, and the fields of
/// that preset to change.
///
/// Every part defaults, so `{}` is a valid request (an empty graph, the layered preset).
///
/// ```
/// use graphlay::json::LayoutRequest;
/// use graphlay::{Direction, RoutingStyle};
///
/// let request: LayoutRequest = serde_json::from_str(r#"{
///     "graph": {"nodes": [{"size": {"width": 80, "height": 40}},
///                         {"size": {"width": 80, "height": 40}}],
///               "edges": [{"source": 0, "target": 1}]},
///     "preset": "process_map",
///     "options": {"routing": {"style": "diagonal"}}
/// }"#)?;
/// let (graph, options) = request.split()?;
/// assert_eq!(options.routing.style, RoutingStyle::Diagonal); // from the patch
/// assert_eq!(options.direction, Direction::TopBottom); // from the preset
/// let l = graphlay::layout(&graph, &options)?;
/// assert!(l.nodes[0].center.y < l.nodes[1].center.y);
///
/// // A bad patch is an error that names the field.
/// let bad: LayoutRequest =
///     serde_json::from_str(r#"{"options": {"routing": {"style": "curvy"}}}"#)?;
/// assert!(bad.split().unwrap_err().starts_with("options.routing.style:"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct LayoutRequest {
    /// The graph to lay out. Nodes and edges are addressed by index, and a `source == target`
    /// edge is a self-loop.
    pub graph: Graph,
    /// Which preset the options start from.
    pub preset: Preset,
    /// Fields of the preset to change, as a partial [`LayoutOptions`] object, deep-merged.
    #[cfg_attr(feature = "schemars", schemars(with = "LayoutOptions"))]
    pub options: Value,
}

impl LayoutRequest {
    /// The graph and the resolved options, ready for [`crate::layout()`] or
    /// [`crate::reroute()`].
    ///
    /// # Errors
    ///
    /// If the patch does not describe valid [`LayoutOptions`]. The message names the field.
    pub fn split(self) -> Result<(Graph, LayoutOptions), String> {
        let options = resolve(LayoutOptions::preset(self.preset), &self.options)?;
        Ok((self.graph, options))
    }
}

/// A tidy-tree request: a rooted tree or forest, and the [`TreeOptions`] fields to change.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct TreeRequest {
    /// The rooted tree or forest to lay out.
    pub graph: Graph,
    /// Fields of [`TreeOptions`] to change, deep-merged over its defaults.
    #[cfg_attr(feature = "schemars", schemars(with = "TreeOptions"))]
    pub options: Value,
}

impl TreeRequest {
    /// The graph and the options, ready for [`crate::tree::layout`].
    ///
    /// # Errors
    ///
    /// If the patch does not describe valid [`TreeOptions`]. The message names the field.
    pub fn split(self) -> Result<(Graph, TreeOptions), String> {
        let options = resolve(TreeOptions::default(), &self.options)?;
        Ok((self.graph, options))
    }
}

fn merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                merge(b.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (b, p) => *b = p.clone(),
    }
}

fn resolve<T: Serialize + serde::de::DeserializeOwned>(
    base: T,
    patch: &Value,
) -> Result<T, String> {
    if patch.is_null() {
        return Ok(base);
    }
    let mut value = serde_json::to_value(base).map_err(|e| e.to_string())?;
    merge(&mut value, patch);
    serde_path_to_error::deserialize(value).map_err(|e| {
        let path = e.path().to_string();
        if path == "." {
            format!("options: {}", e.inner())
        } else {
            format!("options.{path}: {}", e.inner())
        }
    })
}

/// Lays out the graph of a JSON [`LayoutRequest`] and answers with the [`Layout`] as JSON.
pub fn layout(request: &str) -> Result<String, String> {
    let request: LayoutRequest = parse(request)?;
    let (graph, options) = request.split()?;
    let out = crate::layout(&graph, &options).map_err(|e| e.to_string())?;
    to_string(&out)
}

/// Re-routes the edges of a JSON [`LayoutRequest`] over the positions its nodes carry, as
/// [`crate::reroute`] does, and answers with the [`Layout`] as JSON.
pub fn reroute(request: &str) -> Result<String, String> {
    let request: LayoutRequest = parse(request)?;
    let (graph, options) = request.split()?;
    let out = crate::reroute(&graph, &options).map_err(|e| e.to_string())?;
    to_string(&out)
}

/// Lays out the tree of a JSON [`TreeRequest`] and answers with the [`Layout`] as JSON.
pub fn layout_tree(request: &str) -> Result<String, String> {
    let request: TreeRequest = parse(request)?;
    let (graph, options) = request.split()?;
    let out = crate::tree::layout(&graph, &options).map_err(|e| e.to_string())?;
    to_string(&out)
}

/// Measures a JSON [`Layout`] against the JSON [`LayoutRequest`] that produced it and answers
/// with the [`crate::metrics::LayoutMetrics`] as JSON.
pub fn metrics(request: &str, layout: &str) -> Result<String, String> {
    let request: LayoutRequest = parse(request)?;
    let (graph, options) = request.split()?;
    let layout: Layout = parse(layout)?;
    to_string(&crate::metrics::measure(&graph, &options, &layout))
}

/// The [`Layout::fingerprint`] of a JSON [`Layout`].
pub fn fingerprint(layout: &str) -> Result<String, String> {
    let layout: Layout = parse(layout)?;
    Ok(layout.fingerprint())
}

/// The names of the presets, as a request spells them.
pub fn presets() -> Vec<String> {
    Preset::ALL
        .iter()
        .filter_map(|p| serde_json::to_value(p).ok())
        .filter_map(|v| v.as_str().map(String::from))
        .collect()
}

/// The full [`LayoutOptions`] of a preset as JSON, with every field spelled out.
pub fn options(preset: &str) -> Result<String, String> {
    let preset: Preset = serde_json::from_value(Value::String(preset.to_string()))
        .map_err(|_| format!("unknown preset {preset:?}, expected one of {:?}", presets()))?;
    to_string(&LayoutOptions::preset(preset))
}

/// Draws a JSON [`crate::svg::Scene`] with a JSON [`crate::svg::Palette`] to SVG markup. An
/// empty palette means the default one.
#[cfg(feature = "svg")]
pub fn render_scene(scene: &str, palette: &str) -> Result<String, String> {
    let scene: crate::svg::Scene = parse(scene)?;
    let palette = if palette.trim().is_empty() {
        crate::svg::Palette::default()
    } else {
        parse(palette)?
    };
    Ok(crate::svg::render(
        &scene,
        &crate::svg::Theme::new(palette),
        &crate::svg::HeuristicMeasurer::new(),
    ))
}

/// The plain sketch of a JSON [`Layout`] ([`crate::svg::sketch_scene`]) as a JSON
/// [`crate::svg::Scene`], to restyle before [`render_scene`]. Empty options mean the defaults.
#[cfg(feature = "svg")]
pub fn sketch_scene(request: &str, layout: &str, options: &str) -> Result<String, String> {
    let request: LayoutRequest = parse(request)?;
    let layout: Layout = parse(layout)?;
    let options = if options.trim().is_empty() {
        crate::svg::SketchOptions::default()
    } else {
        parse(options)?
    };
    to_string(&crate::svg::sketch_scene(&request.graph, &layout, &options))
}

/// Draws a JSON [`Layout`] of the graph in a JSON [`LayoutRequest`] as a plain sketch
/// ([`crate::svg::sketch()`]) with JSON [`crate::svg::SketchOptions`]. Empty options mean the
/// defaults.
#[cfg(feature = "svg")]
pub fn sketch(request: &str, layout: &str, options: &str) -> Result<String, String> {
    let request: LayoutRequest = parse(request)?;
    let layout: Layout = parse(layout)?;
    let options = if options.trim().is_empty() {
        crate::svg::SketchOptions::default()
    } else {
        parse(options)?
    };
    Ok(crate::svg::sketch(&request.graph, &layout, &options))
}

/// The JSON schemas of the wire types, keyed by type name, for generating typed bindings.
#[cfg(feature = "schemars")]
pub fn schemas() -> String {
    fn one<T: schemars::JsonSchema>() -> Value {
        serde_json::to_value(schemars::schema_for!(T)).unwrap_or_default()
    }
    let mut out = serde_json::Map::new();
    out.insert("LayoutRequest".into(), one::<LayoutRequest>());
    out.insert("TreeRequest".into(), one::<TreeRequest>());
    out.insert("LayoutOptions".into(), one::<LayoutOptions>());
    out.insert("TreeOptions".into(), one::<TreeOptions>());
    out.insert("Layout".into(), one::<Layout>());
    out.insert(
        "LayoutMetrics".into(),
        one::<crate::metrics::LayoutMetrics>(),
    );
    #[cfg(feature = "svg")]
    {
        out.insert("Scene".into(), one::<crate::svg::Scene>());
        out.insert("Palette".into(), one::<crate::svg::Palette>());
        out.insert("SketchOptions".into(), one::<crate::svg::SketchOptions>());
    }
    Value::Object(out).to_string()
}

impl Layout {
    /// The FNV-1a hash of the layout's JSON with every number rounded to a thousandth of a pixel.
    /// The same drawing gives the same string on every target and through every binding.
    pub fn fingerprint(&self) -> String {
        let mut v = serde_json::to_value(self).unwrap_or_default();
        round_numbers(&mut v);
        format!("{:016x}", fnv1a(v.to_string().as_bytes()))
    }
}

/// 64-bit FNV-1a. Unlike `DefaultHasher`, it is the same on every platform and Rust version.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// Rounds every number in `v` to a thousandth, since the last bits of a float differ between
/// targets.
fn round_numbers(v: &mut Value) {
    match v {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                let r = (f * 1000.0).round() / 1000.0;
                // `-0.0` and `0.0` are the same drawing.
                let r = if r == 0.0 { 0.0 } else { r };
                if let Some(m) = serde_json::Number::from_f64(r) {
                    *n = m;
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(round_numbers),
        Value::Object(o) => o.values_mut().for_each(round_numbers),
        _ => {}
    }
}

fn parse<T: DeserializeOwned>(text: &str) -> Result<T, String> {
    serde_json::from_str(text).map_err(|e| e.to_string())
}

fn to_string<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| e.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::{Shape, Size};
    use crate::graph::{Node, RankConstraint};
    use crate::options::Direction;
    use crate::{layout, Layout, RouteKind};

    #[test]
    fn an_empty_request_is_the_layered_preset() {
        let req: LayoutRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(req, LayoutRequest::default());
        let (g, o) = req.split().unwrap();
        assert!(g.nodes.is_empty());
        assert_eq!(o, LayoutOptions::layered_default());

        let text = r#"{
            "graph": {
                "nodes": [{"size": {"width": 80, "height": 40}},
                          {"size": {"width": 80, "height": 40}, "shape": "ellipse"}],
                "edges": [{"source": 0, "target": 1, "weight": 2.5}]
            },
            "options": {"direction": "topbottom"}
        }"#;
        let req: LayoutRequest = serde_json::from_str(text).unwrap();
        assert_eq!(req.graph.nodes[1].shape, Shape::Ellipse);
        assert_eq!(req.graph.edges[0].thickness, 2.0);
        assert_eq!(req.graph.nodes[0].rank, RankConstraint::Free);
        let back: LayoutRequest =
            serde_json::from_str(&serde_json::to_string(&req).unwrap()).unwrap();
        assert_eq!(back, req);
        assert_eq!(req.split().unwrap().1.direction, Direction::TopBottom);
    }

    #[test]
    fn the_patch_is_a_deep_merge_over_the_preset() {
        let req: LayoutRequest = serde_json::from_str(
            r#"{"preset": "process_map",
                "options": {"direction": "leftright", "routing": {"style": "diagonal"}}}"#,
        )
        .unwrap();
        let (_, o) = req.split().unwrap();
        let base = LayoutOptions::process_map();
        assert_eq!(o.direction, Direction::LeftRight);
        assert_eq!(o.routing.style, crate::RoutingStyle::Diagonal);
        assert_eq!(o.routing.bundle_parallel, base.routing.bundle_parallel);
        assert_eq!(o.spacing, base.spacing);
        assert!(o.coords.terminal_centering);
    }

    /// Hub-and-spoke spacing with the process map's flow behaviours.
    #[test]
    fn hub_and_spoke_flow_combines_both_presets() {
        let o = LayoutOptions::preset(Preset::HubAndSpokeFlow);
        let hub = LayoutOptions::hub_and_spoke();
        let flow = LayoutOptions::process_map();
        assert_eq!(o.spacing.layer_gap, hub.spacing.layer_gap);
        assert_eq!(o.compaction.mode, crate::Compaction::Priority);
        assert!(o.coords.terminal_centering);
        assert_eq!(
            o.tuning.coords.pullin_rounds,
            flow.tuning.coords.pullin_rounds
        );
        assert_eq!(o.spacing.reroute_node_gap, flow.spacing.reroute_node_gap);
    }

    #[test]
    fn a_tree_request_patches_the_tree_defaults() {
        let req: TreeRequest =
            serde_json::from_str(r#"{"options": {"direction": "leftright"}}"#).unwrap();
        let (_, o) = req.split().unwrap();
        assert_eq!(o.direction, Direction::LeftRight);
        assert_eq!(o.edge_style, crate::TreeEdgeStyle::Elbow);
    }

    #[test]
    fn an_invalid_patch_names_the_field() {
        let err = |text: &str| {
            serde_json::from_str::<LayoutRequest>(text)
                .unwrap()
                .split()
                .unwrap_err()
        };
        let e = err(r#"{"options": {"routing": {"style": "curvy"}}}"#);
        assert!(e.starts_with("options.routing.style:"), "{e}");
        let e = err(r#"{"options": {"routing": {"styl": "diagonal"}}}"#);
        assert!(e.starts_with("options.routing.styl: unknown field"), "{e}");
        let e = err(r#"{"options": {"direction": "sideways"}}"#);
        assert!(e.starts_with("options.direction:"), "{e}");
        assert!(serde_json::from_str::<LayoutRequest>(r#"{"grap": {}}"#).is_err());
    }

    #[test]
    fn enums_are_lowercase_strings() {
        let g = Graph {
            nodes: vec![Node {
                size: Size::new(1.0, 1.0),
                shape: Shape::Ellipse,
                rank: RankConstraint::First,
                ..Node::default()
            }],
            edges: vec![],
        };
        let text = serde_json::to_string(&g).unwrap();
        assert!(text.contains("\"ellipse\""), "{text}");
        assert!(text.contains("\"first\""), "{text}");
        assert!(serde_json::to_string(&LayoutOptions::layered_default())
            .unwrap()
            .contains("\"leftright\""));
    }

    #[test]
    fn a_layout_round_trips() {
        let req: LayoutRequest = serde_json::from_str(
            r#"{"graph": {"nodes": [{"size": {"width": 80, "height": 40}},
                                    {"size": {"width": 80, "height": 40}}],
                          "edges": [{"source": 0, "target": 1}, {"source": 1, "target": 1}]}}"#,
        )
        .unwrap();
        let (g, o) = req.split().unwrap();
        let l = layout(&g, &o).unwrap();
        let text = serde_json::to_string(&l).unwrap();
        assert!(text.contains("\"selfloop\""), "{text}");
        let back: Layout = serde_json::from_str(&text).unwrap();
        assert_eq!(back, l);
        assert_eq!(back.edges[1].kind, RouteKind::SelfLoop);
    }

    const TWO_BOXES: &str = r#"{"graph": {"nodes": [{"size": {"width": 80, "height": 40}},
        {"size": {"width": 80, "height": 40}}], "edges": [{"source": 0, "target": 1}]},
        "preset": "process_map"}"#;

    #[test]
    fn the_string_entry_points_answer_and_fail_alike() {
        let out = super::layout(TWO_BOXES).unwrap();
        let parsed: Layout = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed.nodes.len(), 2);
        assert_eq!(super::fingerprint(&out).unwrap(), parsed.fingerprint());

        let mut seeded: Value = serde_json::from_str(TWO_BOXES).unwrap();
        for (i, n) in parsed.nodes.iter().enumerate() {
            seeded["graph"]["nodes"][i]["position"] =
                serde_json::json!({"x": n.center.x, "y": n.center.y});
        }
        let rerouted: Layout =
            serde_json::from_str(&super::reroute(&seeded.to_string()).unwrap()).unwrap();
        assert_eq!(rerouted.nodes[1].center, parsed.nodes[1].center);

        let m: Value = serde_json::from_str(&super::metrics(TWO_BOXES, &out).unwrap()).unwrap();
        assert_eq!(m["crossings"], 0);

        assert_eq!(
            super::presets(),
            [
                "layered",
                "process_map",
                "hub_and_spoke",
                "hub_and_spoke_flow"
            ]
        );
        let o: LayoutOptions =
            serde_json::from_str(&super::options("hub_and_spoke").unwrap()).unwrap();
        assert_eq!(o, LayoutOptions::hub_and_spoke());
        assert!(super::options("curvy")
            .unwrap_err()
            .starts_with("unknown preset"));

        let bad = super::layout(r#"{"options": {"routing": {"style": "curvy"}}}"#);
        assert!(bad.unwrap_err().starts_with("options.routing.style"));
        assert!(super::layout("42").unwrap_err().contains("expected"));
        assert!(super::metrics(TWO_BOXES, "{").is_err());
    }

    #[test]
    fn a_tree_request_lays_out_a_forest() {
        let request = r#"{"graph": {"nodes": [{"size": {"width": 80, "height": 40}},
            {"size": {"width": 80, "height": 40}}, {"size": {"width": 80, "height": 40}}],
            "edges": [{"source": 0, "target": 1}, {"source": 0, "target": 2}]}}"#;
        let out: Layout = serde_json::from_str(&super::layout_tree(request).unwrap()).unwrap();
        assert_eq!(out.nodes.len(), 3);
        assert!(out.nodes[0].center.y < out.nodes[1].center.y);
    }

    #[test]
    fn the_fingerprint_rounds_to_a_thousandth() {
        let a: Layout = serde_json::from_str(&super::layout(TWO_BOXES).unwrap()).unwrap();
        let mut b = a.clone();
        b.nodes[0].center.x += 1e-7;
        assert_eq!(a.fingerprint(), b.fingerprint());
        b.nodes[0].center.x += 0.01;
        assert_ne!(a.fingerprint(), b.fingerprint());
    }

    #[cfg(feature = "svg")]
    #[test]
    fn scenes_render_and_sketches_round_trip() {
        let svg = super::render_scene(r#"{"nodes": [], "edges": []}"#, "").unwrap();
        assert!(svg.contains("<svg"));
        assert!(super::render_scene("{}", "{").is_err());
        let out = super::layout(TWO_BOXES).unwrap();
        let svg = super::sketch(TWO_BOXES, &out, r#"{"labels": ["one", "two"]}"#).unwrap();
        assert!(svg.contains("<svg") && svg.contains("two"));
        assert!(super::sketch(TWO_BOXES, &out, r#"{"colour": 1}"#).is_err());
        let scene = super::sketch_scene(TWO_BOXES, &out, r#"{"labels": ["one", "two"]}"#).unwrap();
        assert_eq!(super::render_scene(&scene, "").unwrap(), svg);
    }

    #[cfg(feature = "schemars")]
    #[test]
    fn the_schemas_name_the_wire_types() {
        let v: Value = serde_json::from_str(&super::schemas()).unwrap();
        for k in [
            "LayoutRequest",
            "TreeRequest",
            "LayoutOptions",
            "Layout",
            "LayoutMetrics",
        ] {
            assert!(v.get(k).is_some(), "{k}");
        }
    }
}
