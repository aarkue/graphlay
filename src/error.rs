//! The engine's error type. Every public entry point validates its input up front and reports a
//! bad graph as a [`LayoutError`].

use std::fmt;

use crate::graph::{EdgeId, NodeId};

/// Why an input is not a rooted tree, for [`LayoutError::NotATree`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TreeReason {
    /// A node has an edge to itself, which the tidy-tree layout cannot draw.
    SelfLoop,
    /// A node is the target of more than one edge, so it has two parents.
    MultipleParents,
    /// Every node has a parent, so there is nowhere to start (the input is cyclic).
    NoRoot,
    /// Some nodes are not reachable from any root (a cycle beside a valid tree).
    Unreachable,
}

/// Why a layout could not be produced.
///
/// Non-exhaustive, so a `match` outside the crate needs a wildcard arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayoutError {
    /// An edge names a node index that does not exist.
    EdgeEndpointOutOfRange {
        /// The offending edge.
        edge: EdgeId,
        /// The out-of-range endpoint it names.
        node: u32,
        /// Number of nodes in the input (valid indices are `0..nodes`).
        nodes: u32,
    },
    /// A node's width or height is NaN or infinite.
    NonFiniteSize {
        /// The offending node.
        node: NodeId,
    },
    /// A node's width or height is zero or negative.
    NonPositiveSize {
        /// The offending node.
        node: NodeId,
    },
    /// An edge's layout weight is NaN or infinite.
    NonFiniteWeight {
        /// The offending edge.
        edge: EdgeId,
    },
    /// An edge's layout weight is zero or negative.
    NonPositiveWeight {
        /// The offending edge.
        edge: EdgeId,
    },
    /// A node's warm-start position is NaN or infinite.
    NonFinitePosition {
        /// The offending node.
        node: NodeId,
    },
    /// A node has no warm-start position in a pipeline that requires one ([`crate::reroute`]).
    MissingPosition {
        /// The offending node.
        node: NodeId,
    },
    /// A node declares a `parent`, but this entry point takes flat graphs only
    /// ([`crate::reroute`], [`crate::tree::layout`]).
    NestedNodesUnsupported {
        /// The offending node.
        node: NodeId,
    },
    /// A node's `parent` is not another node of the graph, or its parent chain has a cycle.
    InvalidParent {
        /// The offending node.
        node: NodeId,
    },
    /// The input to [`crate::tree::layout`] is not a rooted tree or forest.
    NotATree {
        /// The offending node, or `NodeId(0)` when the defect is a property of the whole graph
        /// (see [`TreeReason::NoRoot`]).
        node: NodeId,
        /// What is wrong.
        reason: TreeReason,
    },
    /// The input exceeds the caller's [`crate::options::Limits`].
    TooLarge {
        /// Which count was exceeded (`"nodes"`, `"edges"`, ...).
        what: &'static str,
        /// The count the input actually has.
        got: usize,
        /// The configured limit.
        limit: usize,
    },
    /// An edge joins two nodes fixed to the same layer ([`crate::RankConstraint::Fixed`] with
    /// equal values).
    EdgeWithinFixedLayer {
        /// The offending edge.
        edge: EdgeId,
    },
    /// A [`crate::PortHint::Offset`] position is NaN, infinite or outside `-1..=1`.
    InvalidPortOffset {
        /// The offending edge.
        edge: EdgeId,
        /// True for the edge's `source_port`, false for its `target_port`.
        source: bool,
    },
    /// A numeric option is NaN, infinite or negative.
    InvalidOption {
        /// The option's path, as in the JSON form (`"spacing.layer_gap"`).
        field: &'static str,
    },
    /// An internal invariant did not hold. This is a bug.
    Internal(&'static str),
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            LayoutError::EdgeEndpointOutOfRange { edge, node, nodes } => write!(
                f,
                "edge {} references node index {node}, outside 0..{nodes}",
                edge.0
            ),
            LayoutError::NonFiniteSize { node } => {
                write!(f, "node {} has a non-finite size", node.0)
            }
            LayoutError::NonPositiveSize { node } => {
                write!(f, "node {} has a zero or negative size", node.0)
            }
            LayoutError::NonFiniteWeight { edge } => {
                write!(f, "edge {} has a non-finite weight", edge.0)
            }
            LayoutError::NonPositiveWeight { edge } => {
                write!(f, "edge {} has a zero or negative weight", edge.0)
            }
            LayoutError::NonFinitePosition { node } => {
                write!(f, "node {} has a non-finite seed position", node.0)
            }
            LayoutError::MissingPosition { node } => {
                write!(
                    f,
                    "node {} has no seed position; reroute needs one per node",
                    node.0
                )
            }
            LayoutError::NestedNodesUnsupported { node } => write!(
                f,
                "node {} declares a parent; this entry point takes flat graphs only",
                node.0
            ),
            LayoutError::InvalidParent { node } => write!(
                f,
                "node {} has a parent that is not another node, or its parents form a cycle",
                node.0
            ),
            LayoutError::NotATree { node, reason } => match reason {
                TreeReason::SelfLoop => write!(f, "tree layout: self-loop on node {}", node.0),
                TreeReason::MultipleParents => {
                    write!(f, "tree layout: node {} has more than one parent", node.0)
                }
                TreeReason::NoRoot => write!(f, "tree layout: no root (input is cyclic)"),
                TreeReason::Unreachable => {
                    write!(f, "tree layout: unreachable nodes (input is cyclic)")
                }
            },
            LayoutError::TooLarge { what, got, limit } => {
                write!(f, "too many {what}: {got}, limit is {limit}")
            }
            LayoutError::EdgeWithinFixedLayer { edge } => {
                write!(f, "edge {} joins two nodes fixed to the same layer", edge.0)
            }
            LayoutError::InvalidPortOffset { edge, source } => write!(
                f,
                "edge {} has a {} port offset outside -1..=1",
                edge.0,
                if source { "source" } else { "target" }
            ),
            LayoutError::InvalidOption { field } => {
                write!(f, "option `{field}` must be a finite number >= 0")
            }
            LayoutError::Internal(what) => write!(f, "internal layout error: {what}"),
        }
    }
}

impl std::error::Error for LayoutError {}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn display_names_the_offender() {
        let e = LayoutError::EdgeEndpointOutOfRange {
            edge: EdgeId(3),
            node: 9,
            nodes: 4,
        };
        assert_eq!(
            e.to_string(),
            "edge 3 references node index 9, outside 0..4"
        );
        assert_eq!(
            LayoutError::NonPositiveSize { node: NodeId(2) }.to_string(),
            "node 2 has a zero or negative size"
        );
        assert_eq!(
            LayoutError::InvalidPortOffset {
                edge: EdgeId(5),
                source: true
            }
            .to_string(),
            "edge 5 has a source port offset outside -1..=1"
        );
    }

    #[test]
    fn tree_diagnostics_read_as_sentences() {
        let at = |reason| {
            LayoutError::NotATree {
                node: NodeId(7),
                reason,
            }
            .to_string()
        };
        assert_eq!(at(TreeReason::SelfLoop), "tree layout: self-loop on node 7");
        assert_eq!(
            at(TreeReason::MultipleParents),
            "tree layout: node 7 has more than one parent"
        );
        assert_eq!(
            at(TreeReason::NoRoot),
            "tree layout: no root (input is cyclic)"
        );
        assert_eq!(
            at(TreeReason::Unreachable),
            "tree layout: unreachable nodes (input is cyclic)"
        );
    }

    #[test]
    fn is_a_std_error() {
        let boxed: Box<dyn std::error::Error> = Box::new(LayoutError::Internal("x"));
        assert_eq!(boxed.to_string(), "internal layout error: x");
    }
}
