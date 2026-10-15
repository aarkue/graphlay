//! Per-phase straightness profile, showing which phase bends a long edge and which straightens it.
//!
//! Counts `LayoutMetrics::staircase_steps` on each waypoint chain between phases, reading `cross`
//! instead of a polyline. Measurement only, behind the `fixtures` feature.

use crate::error::LayoutError;
use crate::graph::Graph;
use crate::options::LayoutOptions;

use super::coord::chain_jogs;
use super::pipeline::Pipeline;
use super::state::LayeredState;

/// What the long edges looked like after one phase ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseSteps {
    /// Name of the phase that just ran.
    pub phase: &'static str,
    /// Staircase steps over every chain, i.e., consecutive level changes in the same direction.
    pub steps: usize,
    /// Level changes over every chain in either direction. The router draws each as a jog.
    pub jogs: usize,
    /// Chains that run on a single lane from end to end.
    pub level: usize,
    /// Chains with at least one waypoint. The other counts are over these chains.
    pub chains: usize,
}

/// The staircase counts over every waypoint chain of a placed state.
pub fn chain_steps(st: &LayeredState) -> PhaseSteps {
    let mut out = PhaseSteps {
        phase: "",
        steps: 0,
        jogs: 0,
        level: 0,
        chains: 0,
    };
    let cross: Vec<f64> = st.nodes.iter().map(|nd| nd.cross).collect();
    for chain in st.chains.values() {
        if chain.len() < 3
            || !chain[1..chain.len() - 1]
                .iter()
                .any(|&v| st.nodes[v].is_dummy())
        {
            continue;
        }
        out.chains += 1;
        let (jogs, steps) = chain_jogs(&cross, chain);
        out.jogs += jogs;
        out.steps += steps;
        out.level += usize::from(jogs == 0);
    }
    out
}

/// Names the front end that rebuilds the input of `g` before the phase list runs, if any.
///
/// Returns `"parallel-arc bundling"` or `"the isolated-node split"`. When this is `Some`, the
/// real layout differs from what [`phase_profile`] measures.
pub fn front_end(g: &Graph, o: &LayoutOptions) -> Option<&'static str> {
    let mut degree = vec![0usize; g.nodes.len()];
    let mut parallel = false;
    let mut seen: std::collections::BTreeSet<(usize, usize)> = std::collections::BTreeSet::new();
    for e in &g.edges {
        let (a, b) = (e.source.0 as usize, e.target.0 as usize);
        if a >= g.nodes.len() || b >= g.nodes.len() {
            continue;
        }
        if a == b {
            continue;
        }
        degree[a] += 1;
        degree[b] += 1;
        if !seen.insert((a, b)) {
            parallel = true;
        }
    }
    let isolated = degree.iter().filter(|&&d| d == 0).count();
    if o.routing.bundle_parallel && parallel {
        Some("parallel-arc bundling")
    } else if isolated > 0 && isolated < g.nodes.len() {
        Some("the isolated-node split")
    } else {
        None
    }
}

/// Lays `g` out one phase at a time and returns [`chain_steps`] after each phase. Skips the front
/// ends [`front_end`] reports, and returns an empty list for a graph with no nodes.
///
/// # Errors
///
/// Returns the [`LayoutError`] that validation, the limits check or a phase raises.
pub fn phase_profile(g: &Graph, o: &LayoutOptions) -> Result<Vec<PhaseSteps>, LayoutError> {
    g.validate()?;
    o.limits.check(g)?;
    let prepared = super::prepare::ingest(g, o)?;
    if prepared.input.nodes.is_empty() {
        return Ok(vec![]);
    }
    let mut st = LayeredState::new(&prepared.input);
    st.port_hints = prepared.port_hints.clone();
    let pipeline = Pipeline::layered();
    let mut out = Vec::with_capacity(pipeline.phases.len());
    for phase in &pipeline.phases {
        phase.run(&mut st, o)?;
        let mut row = chain_steps(&st);
        row.phase = phase.name();
        out.push(row);
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::Size;
    use crate::graph::{Edge, Node, NodeId};

    /// Two level changes in the same direction are one step. Opposite directions are a detour.
    #[test]
    fn chain_jogs_counts_same_direction_level_changes() {
        assert_eq!(chain_jogs(&[0.0, 10.0, 20.0, 30.0], &[0, 1, 2, 3]), (3, 2));
        assert_eq!(chain_jogs(&[0.0, 10.0, 0.0, 10.0], &[0, 1, 2, 3]), (3, 0));
        assert_eq!(chain_jogs(&[0.0, 0.0, 10.0, 10.0], &[0, 1, 2, 3]), (1, 0));
        // A change under a pixel does not count.
        assert_eq!(chain_jogs(&[0.0, 0.5, 1.0], &[0, 1, 2]), (0, 0));
    }

    /// The profile has one row per phase and finds the chain once `build_chains` has run.
    #[test]
    fn a_chain_counts_its_same_direction_level_changes() {
        let mut g = Graph {
            nodes: vec![
                Node {
                    size: Size::new(40.0, 20.0),
                    ..Node::default()
                };
                4
            ],
            edges: vec![Edge {
                source: NodeId(0),
                target: NodeId(3),
                ..Edge::default()
            }],
        };
        g.edges.push(Edge {
            source: NodeId(0),
            target: NodeId(1),
            ..Edge::default()
        });
        g.edges.push(Edge {
            source: NodeId(1),
            target: NodeId(2),
            ..Edge::default()
        });
        g.edges.push(Edge {
            source: NodeId(2),
            target: NodeId(3),
            ..Edge::default()
        });
        let rows = phase_profile(&g, &LayoutOptions::layered_default()).unwrap();
        assert_eq!(rows.len(), Pipeline::layered().phases.len());
        assert_eq!(rows[0].phase, "break_cycles");
        assert_eq!(rows[0].chains, 0);
        let built = rows.iter().find(|r| r.phase == "build_chains").unwrap();
        assert_eq!(built.chains, 1);
        // The long edge beside a three-hop spine ends up on one lane.
        let last = rows.last().unwrap();
        assert_eq!((last.steps, last.chains), (0, 1));
    }
}
