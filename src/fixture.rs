//! The JSON fixture format documented in `fixtures/FORMAT.md`, one layout problem per file under
//! `fixtures/<kind>/<name>.json`.
//!
//! [`Fixture::to_graph`] turns one into a [`Graph`] and [`LayoutOptions`], and [`load_dir`] reads
//! a tree of them in path order. Available with the `fixtures` cargo feature.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::geometry::{Shape, Size};
use crate::graph::{Edge, Graph, GroupId, Node, NodeId, RankConstraint};
use crate::options::LayoutOptions;
use crate::tree::TreeOptions;

fn default_weight() -> f64 {
    1.0
}
fn default_thickness() -> f64 {
    2.0
}
fn default_shape() -> String {
    "box".to_string()
}
fn default_direction() -> String {
    "TB".to_string()
}

/// Layout mode switches, which [`Fixture::to_graph`] turns into [`LayoutOptions`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureOptions {
    /// Process-map style (DFG-like) spacing and routing.
    #[serde(default)]
    pub flow_edges: bool,
    /// Diagonal instead of orthogonal routing.
    #[serde(default)]
    pub flow_diagonal: bool,
    /// Tidy-tree mode. The input must be a rooted tree or forest.
    #[serde(default)]
    pub tree: bool,
    /// Hub-and-spoke spacing plus cross-axis compaction. Wins over `flow_edges` for spacing.
    #[serde(default)]
    pub compact: bool,
}

/// One node of a fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixtureNode {
    /// Unique string id. Edges address nodes by index, not by id.
    pub id: String,
    /// Display text, possibly empty.
    #[serde(default)]
    pub label: String,
    /// Box width in px.
    pub width: f64,
    /// Box height in px.
    pub height: f64,
    /// `"box"` or `"ellipse"`. Anything else reads as a box.
    #[serde(default = "default_shape")]
    pub shape: String,
    /// `null`, `"first"` or `"last"`, pinning the node to the first or last rank.
    #[serde(default)]
    pub pin: Option<String>,
    /// Optional lane group (e.g., an object type index).
    #[serde(default)]
    pub category: Option<u32>,
}

/// One edge of a fixture. `from == to` is a self-loop, which the engine routes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixtureEdge {
    /// Source node index.
    pub from: usize,
    /// Target node index.
    pub to: usize,
    /// Layout importance (e.g., `1 + ln(count)`), finite and > 0. Defaults to 1.
    #[serde(default = "default_weight")]
    pub weight: f64,
    /// Drawn stroke width in px. Defaults to 2.
    #[serde(default = "default_thickness")]
    pub thickness: f64,
    /// Optional display text for rendering. The layout ignores it.
    #[serde(default)]
    pub label: Option<String>,
    /// Optional `[w, h]` px box to reserve for the label.
    #[serde(default)]
    pub label_size: Option<[f64; 2]>,
    /// Optional group (e.g., the object type of an OC-DFG edge), mapped to [`crate::GroupId`].
    #[serde(default)]
    pub category: Option<u32>,
    /// Rank this edge as `to -> from` while still routing it `from -> to`. Maps to
    /// [`crate::Edge::reverse_for_ranking`].
    #[serde(default)]
    pub reversed: bool,
}

/// One layout problem: the whole contents of a fixture file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fixture {
    /// Unique name, by convention the file stem.
    pub name: String,
    /// Family (`petri`, `dfg`, `tree`, `synthetic`, ...). Free text, used only for grouping.
    pub kind: String,
    /// Free-text provenance (where the graph came from).
    #[serde(default)]
    pub source: String,
    /// `"TB"` (top-bottom, the default) or `"LR"` (left-right), case-insensitive.
    #[serde(default = "default_direction")]
    pub direction: String,
    /// Layout mode switches.
    #[serde(default)]
    pub options: FixtureOptions,
    /// Nodes, in index order.
    pub nodes: Vec<FixtureNode>,
    /// Edges, addressing nodes by index.
    pub edges: Vec<FixtureEdge>,
}

impl Fixture {
    /// Indices (into [`Fixture::edges`]) of the `from == to` edges.
    pub fn self_loop_edges(&self) -> Vec<usize> {
        self.edges
            .iter()
            .enumerate()
            .filter(|(_, e)| e.from == e.to)
            .map(|(i, _)| i)
            .collect()
    }

    /// Indices (into [`Fixture::edges`]) of the edges that are not self-loops, in order.
    pub fn laid_out_edges(&self) -> Vec<usize> {
        self.edges
            .iter()
            .enumerate()
            .filter(|(_, e)| e.from != e.to)
            .map(|(i, _)| i)
            .collect()
    }

    /// The engine input for this fixture: the [`Graph`] and the [`LayoutOptions`] its mode
    /// switches select.
    ///
    /// Edge indices match [`Fixture::edges`] one to one, self-loops included. Edge labels are
    /// dropped and label sizes with a zero extent become `None`.
    pub fn to_graph(&self) -> (Graph, LayoutOptions) {
        let nodes: Vec<Node> = self
            .nodes
            .iter()
            .map(|n| Node {
                size: Size::new(n.width, n.height),
                shape: if n.shape.eq_ignore_ascii_case("ellipse") {
                    Shape::Ellipse
                } else {
                    Shape::Box
                },
                rank: match n.pin.as_deref() {
                    Some("first") => RankConstraint::First,
                    Some("last") => RankConstraint::Last,
                    _ => RankConstraint::Free,
                },
                group: n.category.map(GroupId),
                ..Node::default()
            })
            .collect();
        let edges: Vec<Edge> = self
            .edges
            .iter()
            .map(|e| Edge {
                source: NodeId(e.from as u32),
                target: NodeId(e.to as u32),
                weight: e.weight,
                thickness: e.thickness,
                label_size: e
                    .label_size
                    .map(|[w, h]| Size::new(w, h))
                    .filter(|s| s.width > 0.0 && s.height > 0.0),
                group: e.category.map(GroupId),
                reverse_for_ranking: e.reversed,
                ..Edge::default()
            })
            .collect();
        let options = options_from_flags(
            Some(self.direction.as_str()),
            self.options.flow_edges,
            self.options.flow_diagonal,
            self.options.compact,
        );
        (Graph { nodes, edges }, options)
    }

    /// The tidy-tree options for this fixture (its direction, other fields default).
    pub fn to_tree_options(&self) -> TreeOptions {
        TreeOptions {
            direction: self.to_graph().1.direction,
            ..TreeOptions::default()
        }
    }

    /// Checks the format's rules: unique node ids, in-range edge indices, finite positive weights
    /// and node sizes, finite non-negative thicknesses and label sizes, and a known direction.
    pub fn validate(&self) -> Result<(), String> {
        if !self.direction.eq_ignore_ascii_case("TB") && !self.direction.eq_ignore_ascii_case("LR")
        {
            return Err(format!(
                "{}: direction must be \"TB\" or \"LR\", got {:?}",
                self.name, self.direction
            ));
        }
        let mut ids: Vec<&str> = self.nodes.iter().map(|n| n.id.as_str()).collect();
        ids.sort_unstable();
        if let Some(dup) = ids.windows(2).find(|w| w[0] == w[1]) {
            return Err(format!("{}: duplicate node id {:?}", self.name, dup[0]));
        }
        for (i, n) in self.nodes.iter().enumerate() {
            if !n.width.is_finite() || n.width <= 0.0 || !n.height.is_finite() || n.height <= 0.0 {
                return Err(format!(
                    "{}: node {i} ({}) has a non-finite or non-positive size {}x{}",
                    self.name, n.id, n.width, n.height
                ));
            }
        }
        let node_count = self.nodes.len();
        for (i, e) in self.edges.iter().enumerate() {
            if e.from >= node_count || e.to >= node_count {
                return Err(format!(
                    "{}: edge {i} ({} -> {}) references a node index outside 0..{node_count}",
                    self.name, e.from, e.to
                ));
            }
            if !e.weight.is_finite() || e.weight <= 0.0 {
                return Err(format!(
                    "{}: edge {i} has a non-finite or non-positive weight {}",
                    self.name, e.weight
                ));
            }
            if !e.thickness.is_finite() || e.thickness < 0.0 {
                return Err(format!(
                    "{}: edge {i} has a non-finite or negative thickness {}",
                    self.name, e.thickness
                ));
            }
            if let Some([w, h]) = e.label_size {
                if !w.is_finite() || !h.is_finite() || w < 0.0 || h < 0.0 {
                    return Err(format!("{}: edge {i} has a bad label size", self.name));
                }
            }
        }
        Ok(())
    }
}

/// Reads and validates one fixture file. Error messages start with the file path.
pub fn load_file(path: &Path) -> Result<Fixture, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let fixture: Fixture =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    fixture
        .validate()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(fixture)
}

/// Reads every `*.json` fixture under `dir` recursively, sorted by path, skipping the metric file
/// `baseline.json`. A missing directory yields an empty vector, and an unreadable or invalid file
/// is an error naming it.
pub fn load_dir(dir: &Path) -> Result<Vec<Fixture>, String> {
    let mut paths = Vec::new();
    collect_json_files(dir, &mut paths)?;
    paths.sort_unstable();
    paths.iter().map(|p| load_file(p)).collect()
}

fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "json")
            && path.file_name().is_some_and(|n| n != "baseline.json")
        {
            out.push(path);
        }
    }
    Ok(())
}

/// Serialises a fixture to pretty JSON.
pub fn to_json(fixture: &Fixture) -> Result<String, String> {
    serde_json::to_string_pretty(fixture).map_err(|e| e.to_string())
}

fn options_from_flags(
    direction: Option<&str>,
    flow_edges: bool,
    flow_diagonal: bool,
    compact: bool,
) -> LayoutOptions {
    let mut options = if compact {
        LayoutOptions::hub_and_spoke()
    } else if flow_edges {
        LayoutOptions::process_map()
    } else {
        LayoutOptions::layered_default()
    };
    options.direction = match direction {
        Some(d) if d.eq_ignore_ascii_case("LR") => crate::Direction::LeftRight,
        _ => crate::Direction::TopBottom,
    };
    // `flow_edges` keeps its flow behaviours even when `compact` picked the spacing.
    if flow_edges {
        let flow = LayoutOptions::process_map();
        options.coords.terminal_centering = flow.coords.terminal_centering;
        options.tuning.coords.pullin_rounds = flow.tuning.coords.pullin_rounds;
        options.spacing.reroute_node_gap = flow.spacing.reroute_node_gap;
    }
    if flow_diagonal {
        options.routing.style = crate::RoutingStyle::Diagonal;
    }
    options.compaction.mode = if compact {
        crate::Compaction::Priority
    } else {
        crate::Compaction::None
    };
    options
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn node(id: &str) -> FixtureNode {
        FixtureNode {
            id: id.to_string(),
            label: id.to_string(),
            width: 120.0,
            height: 52.0,
            shape: "box".to_string(),
            pin: None,
            category: None,
        }
    }

    fn edge(from: usize, to: usize) -> FixtureEdge {
        FixtureEdge {
            from,
            to,
            weight: 1.0,
            thickness: 2.0,
            label: None,
            label_size: None,
            category: None,
            reversed: false,
        }
    }

    fn fixture(nodes: Vec<FixtureNode>, edges: Vec<FixtureEdge>) -> Fixture {
        Fixture {
            name: "t".to_string(),
            kind: "synthetic".to_string(),
            source: String::new(),
            direction: "TB".to_string(),
            options: FixtureOptions::default(),
            nodes,
            edges,
        }
    }

    #[test]
    fn to_graph_keeps_self_loops_in_place() {
        let f = fixture(
            vec![node("a"), node("b")],
            vec![edge(0, 0), edge(0, 1), edge(1, 1), edge(1, 0)],
        );
        let (g, _) = f.to_graph();
        assert_eq!(g.edges.len(), 4);
        assert!(g.edges[0].is_self_loop() && g.edges[2].is_self_loop());
        assert_eq!(g.edges[1].source, NodeId(0));
        assert_eq!(g.edges[1].target, NodeId(1));
        assert!(g.validate().is_ok());
    }

    #[test]
    fn to_graph_maps_shapes_and_options() {
        let mut nodes = vec![node("a"), node("b")];
        nodes[1].shape = "ellipse".to_string();
        nodes[1].pin = Some("last".to_string());
        nodes[1].category = Some(3);
        let mut f = fixture(nodes, vec![edge(0, 1)]);
        f.direction = "LR".to_string();
        f.options.flow_edges = true;
        f.options.compact = true;
        f.edges[0].label_size = Some([40.0, 14.0]);
        let (g, o) = f.to_graph();
        assert_eq!(g.nodes[0].shape, Shape::Box);
        assert_eq!(g.nodes[1].shape, Shape::Ellipse);
        assert_eq!(g.nodes[1].rank, RankConstraint::Last);
        assert_eq!(g.nodes[1].group, Some(GroupId(3)));
        assert_eq!(g.edges[0].label_size, Some(Size::new(40.0, 14.0)));
        assert_eq!(o.direction, crate::Direction::LeftRight);
        assert_eq!(o.compaction.mode, crate::Compaction::Priority);
        assert!(o.coords.terminal_centering);
        assert_eq!(f.to_tree_options().direction, crate::Direction::LeftRight);
    }

    #[test]
    fn validate_rejects_bad_input() {
        let mut f = fixture(vec![node("a"), node("b")], vec![edge(0, 5)]);
        assert!(f.validate().is_err(), "out-of-range index must fail");

        f.edges = vec![edge(0, 1)];
        assert!(f.validate().is_ok());

        f.edges[0].weight = 0.0;
        assert!(f.validate().is_err(), "non-positive weight must fail");
        f.edges[0].weight = f64::NAN;
        assert!(f.validate().is_err(), "non-finite weight must fail");
        f.edges[0].weight = 1.0;

        f.nodes[1].id = "a".to_string();
        assert!(f.validate().is_err(), "duplicate ids must fail");
        f.nodes[1].id = "b".to_string();

        f.nodes[1].width = 0.0;
        assert!(f.validate().is_err(), "zero width must fail");
        f.nodes[1].width = 120.0;

        f.direction = "sideways".to_string();
        assert!(f.validate().is_err(), "unknown direction must fail");
    }

    #[test]
    fn minimal_json_parses_with_defaults() {
        let json = r#"{
            "name": "minimal", "kind": "synthetic",
            "nodes": [{"id": "a", "width": 10.0, "height": 10.0},
                      {"id": "b", "width": 10.0, "height": 10.0}],
            "edges": [{"from": 0, "to": 1}]
        }"#;
        let f: Fixture = serde_json::from_str(json).expect("parses");
        assert!(f.validate().is_ok());
        assert_eq!(f.direction, "TB");
        assert_eq!(f.edges[0].weight, 1.0);
        assert_eq!(f.edges[0].thickness, 2.0);
        assert!(!f.options.tree);
    }

    #[test]
    fn json_round_trips() {
        let f = fixture(vec![node("a"), node("b")], vec![edge(0, 1)]);
        let text = to_json(&f).expect("serialises");
        let back: Fixture = serde_json::from_str(&text).expect("parses");
        assert_eq!(f, back);
    }
}
