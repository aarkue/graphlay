//! Tidy tree layout. Parents are centred over their children, and sibling subtrees interleave as
//! closely as their contours allow. A tree edge is drawn border to border, as an org-chart elbow
//! through the row gap or as the straight parent-to-child line ([`TreeEdgeStyle`]).
//!
//! Positions come from the Buchheim-Junger-Leipert algorithm, generalised to variable node sizes.
//! It is linear and iterative, so a 100 000-deep chain lays out on a small stack. Options are in
//! [`TreeOptions`].

use serde::{Deserialize, Serialize};

use crate::error::{LayoutError, TreeReason};
use crate::geometry::shape::{border_toward, Outline};
use crate::geometry::Point;
use crate::graph::{Graph, NodeId};
use crate::layout::{EdgeRoute, Layout, Placement, RouteKind};
use crate::options::Direction;

/// Default of [`TreeOptions::sibling_gap`].
const SIBLING_GAP: f64 = 28.0;
/// Default of [`TreeOptions::subtree_gap`]. Equal to [`SIBLING_GAP`].
const SUBTREE_GAP: f64 = 28.0;
/// Default of [`TreeOptions::row_gap`].
const ROW_GAP: f64 = 56.0;

/// Sentinel for "no such node" in the parent / thread / sibling tables.
const NONE: usize = usize::MAX;

/// Which row a node is drawn in, along the axis the tree grows.
///
/// The contour packing pairs subtrees up by depth, so other alignments would need a different
/// contour walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LevelAlignment {
    /// One row per depth, shared by the whole drawing. A node of depth `d` sits in row `d`, and
    /// row `d` is as tall as the tallest node of that depth.
    #[default]
    ByDepth,
}

/// How a tree edge is drawn between parent and child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum TreeEdgeStyle {
    /// The straight parent-to-child line, clipped to both borders. A wide fan's outer edges run
    /// diagonally over the boxes beside the parent.
    Straight,
    /// The org-chart elbow through the middle of the row gap, where no node is. This is the
    /// default.
    #[default]
    Elbow,
}

/// What steers a tidy-tree layout.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct TreeOptions {
    /// Which way the tree grows. `TopBottom` puts the roots at the top, `LeftRight` on the left.
    /// Defaults to `TopBottom`, unlike [`crate::LayoutOptions::direction`].
    pub direction: Direction,
    /// Clearance between the boxes of two neighbouring siblings, in px. Defaults to `28`.
    pub sibling_gap: f64,
    /// Clearance between two nodes that face each other on the contours of different subtrees,
    /// in px. Defaults to `28`.
    pub subtree_gap: f64,
    /// Gap between two depth rows, on the axis the tree grows along, in px. Defaults to `56`.
    pub row_gap: f64,
    /// Which row a node is drawn in. See [`LevelAlignment`].
    pub level_alignment: LevelAlignment,
    /// How a parent-to-child edge is drawn. See [`TreeEdgeStyle`].
    pub edge_style: TreeEdgeStyle,
}

impl Default for TreeOptions {
    /// The gap constants and [`TreeEdgeStyle::Elbow`].
    fn default() -> Self {
        TreeOptions {
            direction: Direction::TopBottom,
            sibling_gap: SIBLING_GAP,
            subtree_gap: SUBTREE_GAP,
            row_gap: ROW_GAP,
            level_alignment: LevelAlignment::ByDepth,
            edge_style: TreeEdgeStyle::default(),
        }
    }
}

/// Lays out a rooted tree or forest with parents centred over their children, sibling subtrees
/// packed as closely as their contours allow, and one row per depth.
///
/// Every edge is a [`RouteKind::Direct`] polyline shaped by [`TreeOptions::edge_style`].
/// [`Placement::rank`] is the node's depth and [`Placement::order`] its index among the nodes of
/// that depth. Port hints and groups are ignored.
///
/// ```
/// use graphlay::{tree, GraphBuilder, Size, TreeOptions};
///
/// let mut b = GraphBuilder::new();
/// let ceo = b.add_box(Size::new(100.0, 40.0));
/// let cto = b.add_box(Size::new(100.0, 40.0));
/// let cfo = b.add_box(Size::new(100.0, 40.0));
/// b.connect(ceo, cto);
/// b.connect(ceo, cfo);
/// let l = tree::layout(&b.build(), &TreeOptions::default())?;
///
/// let (root, left, right) = (l.nodes[0].center, l.nodes[1].center, l.nodes[2].center);
/// // Top to bottom by default: the children share the row below, the root centred over them.
/// assert_eq!(left.y, right.y);
/// assert!(left.y > root.y);
/// assert!((root.x - (left.x + right.x) / 2.0).abs() < 1e-9);
/// // The default edge is the elbow: down, across the row gap, down.
/// assert_eq!(l.edges[0].points.len(), 4);
/// # Ok::<(), graphlay::LayoutError>(())
/// ```
///
/// # Errors
///
/// - [`LayoutError::InvalidOption`] for a gap that is negative or not finite.
/// - Whatever [`Graph::validate`] rejects.
/// - [`LayoutError::NestedNodesUnsupported`] for a graph with compounds.
/// - [`LayoutError::NotATree`] if the input is not a rooted tree or forest: a node with two
///   parents, a self-loop, or a cycle.
pub fn layout(g: &Graph, o: &TreeOptions) -> Result<Layout, LayoutError> {
    for (field, v) in [
        ("sibling_gap", o.sibling_gap),
        ("subtree_gap", o.subtree_gap),
        ("row_gap", o.row_gap),
    ] {
        if !(v.is_finite() && v >= 0.0) {
            return Err(LayoutError::InvalidOption { field });
        }
    }
    g.validate()?;
    crate::reject_compounds(g)?;
    let n = g.nodes.len();
    if n == 0 {
        return Ok(Layout::default());
    }

    // A virtual root over the real roots lets the same contour walk pack a forest.
    let vroot = n;
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
    let mut indeg = vec![0usize; n];
    for e in &g.edges {
        let (a, b) = (e.source.index(), e.target.index());
        if a == b {
            return Err(LayoutError::NotATree {
                node: e.source,
                reason: TreeReason::SelfLoop,
            });
        }
        children[a].push(b);
        indeg[b] += 1;
        if indeg[b] > 1 {
            return Err(LayoutError::NotATree {
                node: e.target,
                reason: TreeReason::MultipleParents,
            });
        }
    }

    // LR swaps the axes, so siblings pack along y and depth runs along x.
    let lr = o.direction == Direction::LeftRight;
    let cross = |i: usize| {
        if lr {
            g.nodes[i].size.height
        } else {
            g.nodes[i].size.width
        }
    };
    let along = |i: usize| {
        if lr {
            g.nodes[i].size.width
        } else {
            g.nodes[i].size.height
        }
    };

    children[vroot] = (0..n).filter(|&i| indeg[i] == 0).collect();
    if children[vroot].is_empty() {
        return Err(LayoutError::NotATree {
            node: NodeId(0),
            reason: TreeReason::NoRoot,
        });
    }

    // Iterative because depth is caller data and recursion would overflow the 1MB wasm stack.
    let mut depth = vec![0usize; n];
    let mut seen = vec![false; n];
    let mut stack: Vec<(usize, usize)> = children[vroot].iter().map(|&r| (r, 0)).collect();
    while let Some((i, d)) = stack.pop() {
        seen[i] = true;
        depth[i] = d;
        for &c in &children[i] {
            stack.push((c, d + 1));
        }
    }
    if let Some(node) = seen.iter().position(|s| !s) {
        return Err(LayoutError::NotATree {
            node: NodeId(node as u32),
            reason: TreeReason::Unreachable,
        });
    }

    let mut tidy = Tidy::new(children, n, |i| cross(i) / 2.0, o);
    let pos = tidy.run(vroot);

    let max_depth = depth.iter().copied().max().unwrap_or(0);
    let mut row_size = vec![0.0f64; max_depth + 1];
    for i in 0..n {
        row_size[depth[i]] = row_size[depth[i]].max(along(i));
    }
    let mut row_at = vec![0.0f64; max_depth + 1];
    let mut acc = 0.0;
    for d in 0..=max_depth {
        row_at[d] = acc + row_size[d] / 2.0;
        acc += row_size[d] + o.row_gap;
    }

    let centers: Vec<Point> = (0..n)
        .map(|i| {
            if lr {
                Point::new(row_at[depth[i]], pos[i])
            } else {
                Point::new(pos[i], row_at[depth[i]])
            }
        })
        .collect();

    // No reordering pass, so the order is the count of earlier nodes in the same row.
    let mut row_seen = vec![0u32; max_depth + 1];
    let nodes: Vec<Placement> = (0..n)
        .map(|i| {
            let order = row_seen[depth[i]];
            row_seen[depth[i]] += 1;
            Placement {
                center: centers[i],
                size: g.nodes[i].size,
                rank: Some(depth[i] as i32),
                order,
            }
        })
        .collect();

    let outline = |i: usize| Outline {
        center: (centers[i].x, centers[i].y),
        size: (g.nodes[i].size.width, g.nodes[i].size.height),
        shape: g.nodes[i].shape,
    };
    // The middle of the gap below row `d` is guaranteed to hold no node.
    let gap_mid = |d: usize| row_at[d] + row_size[d] / 2.0 + o.row_gap / 2.0;
    let edges: Vec<EdgeRoute> = g
        .edges
        .iter()
        .map(|e| {
            let (a, b) = (e.source.index(), e.target.index());
            let points = match o.edge_style {
                TreeEdgeStyle::Straight => {
                    straight_edge(outline(a), outline(b), centers[a], centers[b])
                }
                TreeEdgeStyle::Elbow => elbow_edge(
                    outline(a),
                    outline(b),
                    centers[a],
                    centers[b],
                    gap_mid(depth[a]),
                    lr,
                ),
            };
            let Some(points) = points else {
                return EdgeRoute {
                    kind: RouteKind::Unrouted,
                    ..EdgeRoute::default()
                };
            };
            EdgeRoute::routed(points, RouteKind::Direct, &nodes[a], &nodes[b], None)
        })
        .collect();

    let mut layout = Layout {
        nodes,
        edges,
        layer_count: (max_depth + 1) as u32,
        ..Layout::default()
    };
    layout.recompute_bounds();
    Ok(layout)
}

/// The straight parent-to-child line, clipped to both outlines.
///
/// `None` for coincident centres, which give no direction to clip along.
fn straight_edge(from: Outline, to: Outline, pa: Point, pb: Point) -> Option<Vec<Point>> {
    let a = border_toward(from, (pb.x, pb.y))?;
    let b = border_toward(to, (pa.x, pa.y))?;
    Some(vec![Point::new(a.0, a.1), Point::new(b.0, b.1)])
}

/// The org-chart elbow: out of the middle of the parent's leading border, across the middle of the
/// row gap, and into the middle of the child's trailing border.
///
/// A child directly below its parent gets a two-point line.
fn elbow_edge(
    from: Outline,
    to: Outline,
    pa: Point,
    pb: Point,
    gap_mid: f64,
    lr: bool,
) -> Option<Vec<Point>> {
    // `lr` swaps the axes, so the tree grows along x and the rows stack along y.
    let along = |p: Point| if lr { p.x } else { p.y };
    let cross = |p: Point| if lr { p.y } else { p.x };
    let point = |along_v: f64, cross_v: f64| {
        if lr {
            Point::new(along_v, cross_v)
        } else {
            Point::new(cross_v, along_v)
        }
    };
    // Aiming one pixel along the growth axis hits the middle of the leading or trailing face.
    let ahead = point(along(pa) + 1.0, cross(pa));
    let behind = point(along(pb) - 1.0, cross(pb));
    let exit = border_toward(from, (ahead.x, ahead.y))?;
    let entry = border_toward(to, (behind.x, behind.y))?;
    let (exit, entry) = (Point::new(exit.0, exit.1), Point::new(entry.0, entry.1));
    if (cross(pa) - cross(pb)).abs() < 0.5 {
        return Some(vec![exit, entry]); // straight down, where an elbow would add two dead corners
    }
    Some(vec![
        exit,
        point(gap_mid, cross(pa)),
        point(gap_mid, cross(pb)),
        entry,
    ])
}

/// The Buchheim-Junger-Leipert tidy-tree state, generalised to variable node sizes.
///
/// Vectors are indexed by node, plus one slot for the virtual root. `thread` points from a leaf
/// to the next node on its contour one row down, which keeps the walk linear.
struct Tidy {
    children: Vec<Vec<usize>>,
    parent: Vec<usize>,
    /// The sibling immediately to the left, or [`NONE`] for a first child.
    left_sib: Vec<usize>,
    /// 1-based index among the siblings, which is how many subtrees a deferred shift is spread
    /// over.
    number: Vec<usize>,
    /// Half the node's cross-axis size. `0` for the virtual root, which is not drawn.
    half: Vec<f64>,
    prelim: Vec<f64>,
    modifier: Vec<f64>,
    shift: Vec<f64>,
    change: Vec<f64>,
    thread: Vec<usize>,
    ancestor: Vec<usize>,
    sibling_gap: f64,
    subtree_gap: f64,
}

/// One node of the iterative first walk: which node, how many of its children have been walked,
/// and the default ancestor carried across them.
struct Frame {
    v: usize,
    next_child: usize,
    default_ancestor: usize,
}

impl Tidy {
    fn new(
        children: Vec<Vec<usize>>,
        n: usize,
        half_of: impl Fn(usize) -> f64,
        o: &TreeOptions,
    ) -> Tidy {
        let len = children.len();
        let mut parent = vec![NONE; len];
        let mut left_sib = vec![NONE; len];
        let mut number = vec![1usize; len];
        for (v, kids) in children.iter().enumerate() {
            for (k, &c) in kids.iter().enumerate() {
                parent[c] = v;
                number[c] = k + 1;
                if k > 0 {
                    left_sib[c] = kids[k - 1];
                }
            }
        }
        let half = (0..len)
            .map(|i| if i < n { half_of(i) } else { 0.0 })
            .collect();
        Tidy {
            children,
            parent,
            left_sib,
            number,
            half,
            prelim: vec![0.0; len],
            modifier: vec![0.0; len],
            shift: vec![0.0; len],
            change: vec![0.0; len],
            thread: vec![NONE; len],
            ancestor: (0..len).collect(),
            sibling_gap: o.sibling_gap,
            subtree_gap: o.subtree_gap,
        }
    }

    /// Both walks, returning the cross-axis centre of every node, shifted so the leftmost node box
    /// starts at `0`.
    fn run(&mut self, vroot: usize) -> Vec<f64> {
        self.first_walk(vroot);
        let mut pos = self.second_walk(vroot);
        let min = (0..pos.len() - 1)
            .map(|i| pos[i] - self.half[i])
            .fold(f64::INFINITY, f64::min);
        if min.is_finite() {
            for p in pos.iter_mut() {
                *p -= min;
            }
        }
        pos.truncate(pos.len() - 1); // drop the virtual root
        pos
    }

    /// Leftmost node one row down on this node's left contour.
    fn next_left(&self, v: usize) -> usize {
        self.children[v].first().copied().unwrap_or(self.thread[v])
    }

    /// Rightmost node one row down on this node's right contour.
    fn next_right(&self, v: usize) -> usize {
        self.children[v].last().copied().unwrap_or(self.thread[v])
    }

    /// Post-order walk that gives every node a preliminary position, pushing each subtree clear of
    /// the ones already placed to its left.
    fn first_walk(&mut self, root: usize) {
        let mut stack = vec![Frame {
            v: root,
            next_child: 0,
            default_ancestor: self.children[root].first().copied().unwrap_or(root),
        }];
        while let Some(frame) = stack.last_mut() {
            let v = frame.v;
            if frame.next_child > 0 {
                let w = self.children[v][frame.next_child - 1];
                let da = frame.default_ancestor;
                let next = self.apportion(w, da);
                if let Some(frame) = stack.last_mut() {
                    frame.default_ancestor = next;
                }
            }
            let Some(frame) = stack.last_mut() else { break };
            if frame.next_child < self.children[v].len() {
                let c = self.children[v][frame.next_child];
                frame.next_child += 1;
                stack.push(Frame {
                    v: c,
                    next_child: 0,
                    default_ancestor: self.children[c].first().copied().unwrap_or(c),
                });
                continue;
            }
            let midpoint = if self.children[v].is_empty() {
                None
            } else {
                self.execute_shifts(v);
                let first = self.children[v][0];
                let last = self.children[v][self.children[v].len() - 1];
                Some((self.prelim[first] + self.prelim[last]) / 2.0)
            };
            let w = self.left_sib[v];
            if w == NONE {
                self.prelim[v] = midpoint.unwrap_or(0.0);
            } else {
                self.prelim[v] = self.prelim[w] + self.half[w] + self.sibling_gap + self.half[v];
                if let Some(mid) = midpoint {
                    self.modifier[v] = self.prelim[v] - mid;
                }
            }
            stack.pop();
        }
    }

    /// Push `v`'s subtree right until its left contour clears every subtree already placed under
    /// the same parent, then thread the contours together.
    fn apportion(&mut self, v: usize, mut default_ancestor: usize) -> usize {
        let w = self.left_sib[v];
        if w == NONE {
            return default_ancestor;
        }
        // `*ip`/`*op` walk v's inner (left) and outer (right) contour, `*im`/`*om` the left
        // neighbour's inner (right) contour and the leftmost sibling's outer (left) one.
        let (mut vip, mut vop, mut vim) = (v, v, w);
        let mut vom = self.children[self.parent[v]][0];
        let (mut sip, mut sop) = (self.modifier[vip], self.modifier[vop]);
        let (mut sim, mut som) = (self.modifier[vim], self.modifier[vom]);
        loop {
            let (nim, nip) = (self.next_right(vim), self.next_left(vip));
            if nim == NONE || nip == NONE {
                break;
            }
            // Threading keeps outer contours at least as deep as inner ones, so these are never
            // `NONE`. Breaking instead of indexing keeps a violated invariant from panicking.
            let (nom, nop) = (self.next_left(vom), self.next_right(vop));
            if nom == NONE || nop == NONE {
                break;
            }
            vim = nim;
            vip = nip;
            vom = nom;
            vop = nop;
            self.ancestor[vop] = v;
            let shift =
                (self.prelim[vim] + sim + self.half[vim] + self.subtree_gap + self.half[vip])
                    - (self.prelim[vip] + sip);
            if shift > 0.0 {
                let anchor = self.ancestor_or(vim, v, default_ancestor);
                self.move_subtree(anchor, v, shift);
                sip += shift;
                sop += shift;
            }
            sim += self.modifier[vim];
            sip += self.modifier[vip];
            som += self.modifier[vom];
            sop += self.modifier[vop];
        }
        if self.next_right(vim) != NONE && self.next_right(vop) == NONE {
            self.thread[vop] = self.next_right(vim);
            self.modifier[vop] += sim - sop;
        }
        if self.next_left(vip) != NONE && self.next_left(vom) == NONE {
            self.thread[vom] = self.next_left(vip);
            self.modifier[vom] += sip - som;
            default_ancestor = v;
        }
        default_ancestor
    }

    /// The greatest distinct ancestor of `vim` that is a sibling of `v`, or the default one when
    /// `vim` sits in a subtree that was never threaded to this parent.
    fn ancestor_or(&self, vim: usize, v: usize, default_ancestor: usize) -> usize {
        let a = self.ancestor[vim];
        if a != NONE && self.parent[a] == self.parent[v] {
            a
        } else {
            default_ancestor
        }
    }

    /// Move `wp`'s subtree `shift` px right and defer spreading the subtrees between `wm` and `wp`
    /// to [`Tidy::execute_shifts`].
    fn move_subtree(&mut self, wm: usize, wp: usize, shift: f64) {
        let subtrees = self.number[wp] as f64 - self.number[wm] as f64;
        if subtrees <= 0.0 {
            return; // not siblings in order, so nothing to spread over
        }
        self.change[wp] -= shift / subtrees;
        self.shift[wp] += shift;
        self.change[wm] += shift / subtrees;
        self.prelim[wp] += shift;
        self.modifier[wp] += shift;
    }

    /// Apply the shifts [`Tidy::move_subtree`] deferred, right to left, in one linear pass.
    fn execute_shifts(&mut self, v: usize) {
        let (mut shift, mut change) = (0.0, 0.0);
        for k in (0..self.children[v].len()).rev() {
            let w = self.children[v][k];
            self.prelim[w] += shift;
            self.modifier[w] += shift;
            change += self.change[w];
            shift += self.shift[w] + change;
        }
    }

    /// Sum the modifiers down the tree into an absolute cross-axis centre per node.
    fn second_walk(&self, root: usize) -> Vec<f64> {
        let mut pos = vec![0.0f64; self.children.len()];
        let mut stack = vec![(root, -self.prelim[root])];
        while let Some((v, m)) = stack.pop() {
            pos[v] = self.prelim[v] + m;
            let below = m + self.modifier[v];
            for k in 0..self.children[v].len() {
                stack.push((self.children[v][k], below));
            }
        }
        pos
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::geometry::fp;
    use crate::geometry::{Shape, Size};
    use crate::graph::{Edge, GraphBuilder, Node};

    const OP: (f64, f64) = (56.0, 56.0);
    const LEAF: (f64, f64) = (132.0, 48.0);

    fn node((width, height): (f64, f64)) -> Node {
        Node {
            size: Size::new(width, height),
            ..Node::default()
        }
    }

    fn graph(nodes: Vec<Node>, edges: Vec<(usize, usize)>) -> Graph {
        let mut b = GraphBuilder::new();
        let ids: Vec<NodeId> = nodes.into_iter().map(|n| b.add_node(n)).collect();
        for (a, c) in edges {
            b.add_edge(Edge::new(
                ids.get(a).copied().unwrap_or(NodeId(a as u32)),
                ids.get(c).copied().unwrap_or(NodeId(c as u32)),
            ));
        }
        b.build()
    }

    fn tb() -> TreeOptions {
        TreeOptions::default()
    }

    fn elbow() -> TreeOptions {
        TreeOptions {
            edge_style: TreeEdgeStyle::Elbow,
            ..TreeOptions::default()
        }
    }

    fn straight() -> TreeOptions {
        TreeOptions {
            edge_style: TreeEdgeStyle::Straight,
            ..TreeOptions::default()
        }
    }

    fn tree(g: &Graph) -> Layout {
        layout(g, &tb()).unwrap()
    }

    fn span(l: &Layout, i: usize, width: f64) -> (f64, f64) {
        (
            l.nodes[i].center.x - width / 2.0,
            l.nodes[i].center.x + width / 2.0,
        )
    }

    /// Pairs of node boxes that overlap on both axes.
    fn overlapping_pairs(g: &Graph, l: &Layout) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for i in 0..l.nodes.len() {
            for j in i + 1..l.nodes.len() {
                let (a, b) = (&l.nodes[i], &l.nodes[j]);
                let (aw, ah) = (g.nodes[i].size.width / 2.0, g.nodes[i].size.height / 2.0);
                let (bw, bh) = (g.nodes[j].size.width / 2.0, g.nodes[j].size.height / 2.0);
                let ox =
                    (a.center.x + aw).min(b.center.x + bw) - (a.center.x - aw).max(b.center.x - bw);
                let oy =
                    (a.center.y + ah).min(b.center.y + bh) - (a.center.y - ah).max(b.center.y - bh);
                if ox > 1e-6 && oy > 1e-6 {
                    out.push((i, j));
                }
            }
        }
        out
    }

    #[test]
    fn parent_is_centred_over_its_children() {
        let g = graph(
            vec![node(OP), node(LEAF), node(LEAF), node(LEAF)],
            vec![(0, 1), (0, 2), (0, 3)],
        );
        let l = tree(&g);
        let mid = (l.nodes[1].center.x + l.nodes[3].center.x) / 2.0;
        assert!((l.nodes[0].center.x - mid).abs() < 1e-6);
        assert!((l.nodes[0].center.x - l.nodes[2].center.x).abs() < 1e-6);
    }

    #[test]
    fn nested_parents_are_each_centred() {
        let g = graph(
            vec![node(OP), node(LEAF), node(OP), node(LEAF), node(LEAF)],
            vec![(0, 1), (0, 2), (2, 3), (2, 4)],
        );
        let l = tree(&g);
        let inner = (l.nodes[3].center.x + l.nodes[4].center.x) / 2.0;
        assert!((l.nodes[2].center.x - inner).abs() < 1e-6);
        let outer = (l.nodes[1].center.x + l.nodes[2].center.x) / 2.0;
        assert!((l.nodes[0].center.x - outer).abs() < 1e-6);
    }

    /// Every parent in a deeper tree is centred over its children.
    #[test]
    fn every_parent_is_centred_over_its_children() {
        let g = graph(
            vec![
                node(OP),
                node(OP),
                node(LEAF),
                node(OP),
                node((300.0, 48.0)),
                node(LEAF),
                node(OP),
                node(LEAF),
                node((40.0, 48.0)),
            ],
            vec![
                (0, 1),
                (0, 6),
                (1, 2),
                (1, 3),
                (3, 4),
                (3, 5),
                (6, 7),
                (6, 8),
            ],
        );
        let l = tree(&g);
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); g.nodes.len()];
        for e in &g.edges {
            children[e.source.index()].push(e.target.index());
        }
        for (i, kids) in children.iter().enumerate() {
            if kids.is_empty() {
                continue;
            }
            let first = l.nodes[kids[0]].center.x;
            let last = l.nodes[kids[kids.len() - 1]].center.x;
            assert!(
                (l.nodes[i].center.x - (first + last) / 2.0).abs() < 1e-6,
                "node {i} is not centred over its children"
            );
        }
    }

    /// A deep subtree that is wide at the bottom nests under a shallow one that is wide at the top.
    #[test]
    fn contours_let_a_deep_subtree_nest_beside_a_shallow_one() {
        let wide = (400.0, 48.0);
        // Deep: op -> op -> two wide leaves, so it is 20px wide at rows 1-2 and 828px at row 3.
        let deep_nodes = vec![
            node((20.0, 48.0)),
            node((20.0, 48.0)),
            node(wide),
            node(wide),
        ];
        let deep_edges = vec![(0, 1), (1, 2), (1, 3)];
        // Shallow: op -> two wide leaves, so it is 828px wide at row 2 and stops there.
        let shallow_nodes = vec![node((20.0, 48.0)), node(wide), node(wide)];
        let shallow_edges = vec![(0, 1), (0, 2)];

        let deep_alone = tree(&graph(deep_nodes.clone(), deep_edges.clone()))
            .bounds
            .width();
        let shallow_alone = tree(&graph(shallow_nodes.clone(), shallow_edges.clone()))
            .bounds
            .width();

        // Both under one root: node 0 is the root, 1..=4 the deep subtree, 5..=7 the shallow one.
        let mut nodes = vec![node(OP)];
        nodes.extend(deep_nodes);
        nodes.extend(shallow_nodes);
        let mut edges = vec![(0, 1), (0, 5)];
        edges.extend(deep_edges.iter().map(|&(a, b)| (a + 1, b + 1)));
        edges.extend(shallow_edges.iter().map(|&(a, b)| (a + 5, b + 5)));
        let l = tree(&graph(nodes, edges));
        let together = l.bounds.width();

        let disjoint = deep_alone + SIBLING_GAP + shallow_alone;
        assert!(
            together < 0.8 * disjoint,
            "contours did not pack: {together} vs {disjoint} laid out apart"
        );
    }

    /// No two boxes touch, at any depth, in either direction.
    #[test]
    fn a_packed_tree_has_no_overlapping_boxes() {
        // A lopsided tree: wide leaves at several depths beside chains of narrow operators.
        let mut nodes = vec![node(OP)];
        let mut edges = Vec::new();
        let mut frontier = vec![0usize];
        for depth in 0..4 {
            let mut next = Vec::new();
            for &p in &frontier {
                for k in 0..3 {
                    let size = if (p + k + depth) % 2 == 0 {
                        (200.0, 48.0)
                    } else {
                        (32.0, 64.0)
                    };
                    nodes.push(node(size));
                    let child = nodes.len() - 1;
                    edges.push((p, child));
                    if depth < 3 && k != 1 {
                        next.push(child);
                    }
                }
            }
            frontier = next;
        }
        let g = graph(nodes, edges);
        for direction in [Direction::TopBottom, Direction::LeftRight] {
            let l = layout(
                &g,
                &TreeOptions {
                    direction,
                    ..TreeOptions::default()
                },
            )
            .unwrap();
            let bad = overlapping_pairs(&g, &l);
            assert!(bad.is_empty(), "{direction:?}: overlapping boxes {bad:?}");
        }
    }

    /// A parent wider than its children hangs past their block, and does so asymmetrically when the
    /// children differ in width. Its box, not its centre, is what must clear the next subtree.
    #[test]
    fn wide_parent_with_asymmetric_children_clears_its_sibling() {
        let g = graph(
            vec![
                node(OP),
                node((300.0, 48.0)),
                node(LEAF),
                node((200.0, 48.0)),
                node((20.0, 48.0)),
            ],
            vec![(0, 1), (0, 2), (1, 3), (1, 4)],
        );
        let l = tree(&g);
        let wide = span(&l, 1, 300.0);
        let sibling = span(&l, 2, 132.0);
        assert!(
            sibling.0 - wide.1 >= SIBLING_GAP - 1e-6,
            "gap {} between {wide:?} and {sibling:?}",
            sibling.0 - wide.1
        );
    }

    #[test]
    fn siblings_clear_each_other() {
        let g = graph(
            vec![node(OP), node(LEAF), node(LEAF), node(LEAF)],
            vec![(0, 1), (0, 2), (0, 3)],
        );
        let l = tree(&g);
        let mut spans: Vec<(f64, f64)> = (1..4).map(|i| span(&l, i, LEAF.0)).collect();
        spans.sort_unstable_by(crate::geometry::stats::pair_order);
        for w in spans.windows(2) {
            assert!(w[1].0 - w[0].1 >= SIBLING_GAP - 1e-6);
        }
    }

    /// Facing contours of different subtrees are held apart by `subtree_gap`.
    #[test]
    fn the_subtree_gap_separates_facing_contours() {
        // 1 and 4 are siblings. 3 (under 1) and 5 (under 4) face each other one row down.
        let g = graph(
            vec![
                node(OP),
                node((20.0, 48.0)),
                node((20.0, 48.0)),
                node((200.0, 48.0)),
                node((20.0, 48.0)),
                node((200.0, 48.0)),
            ],
            vec![(0, 1), (0, 4), (1, 2), (1, 3), (4, 5)],
        );
        let tight = layout(&g, &tb()).unwrap();
        let loose = layout(
            &g,
            &TreeOptions {
                subtree_gap: 128.0,
                ..TreeOptions::default()
            },
        )
        .unwrap();
        let gap = |l: &Layout| (l.nodes[5].center.x - 100.0) - (l.nodes[3].center.x + 100.0);
        assert!((gap(&tight) - SUBTREE_GAP).abs() < 1e-6, "{}", gap(&tight));
        assert!((gap(&loose) - 128.0).abs() < 1e-6, "{}", gap(&loose));
    }

    #[test]
    fn rows_are_one_gap_apart() {
        let g = graph(vec![node(OP), node(LEAF)], vec![(0, 1)]);
        let l = tree(&g);
        let expected = OP.1 / 2.0 + ROW_GAP + LEAF.1 / 2.0;
        assert!((l.nodes[1].center.y - l.nodes[0].center.y - expected).abs() < 1e-6);
    }

    /// The gap options default to the module constants.
    #[test]
    fn the_gaps_are_tunable() {
        let g = graph(vec![node(OP), node(LEAF)], vec![(0, 1)]);
        let o = TreeOptions {
            row_gap: 200.0,
            ..TreeOptions::default()
        };
        let l = layout(&g, &o).unwrap();
        let expected = OP.1 / 2.0 + 200.0 + LEAF.1 / 2.0;
        assert!((l.nodes[1].center.y - l.nodes[0].center.y - expected).abs() < 1e-6);
    }

    #[test]
    fn lr_swaps_the_axes() {
        let g = graph(vec![node(OP), node(LEAF)], vec![(0, 1)]);
        let l = layout(
            &g,
            &TreeOptions {
                direction: Direction::LeftRight,
                ..TreeOptions::default()
            },
        )
        .unwrap();
        assert!(l.nodes[1].center.x > l.nodes[0].center.x);
        assert!((l.nodes[1].center.y - l.nodes[0].center.y).abs() < 1e-6);
    }

    #[test]
    fn a_forest_lays_its_roots_side_by_side() {
        let g = graph(
            vec![node(OP), node(LEAF), node(OP), node(LEAF)],
            vec![(0, 1), (2, 3)],
        );
        let l = tree(&g);
        assert!((l.nodes[0].center.y - l.nodes[2].center.y).abs() < 1e-6);
        let (a, b) = (span(&l, 1, LEAF.0), span(&l, 3, LEAF.0));
        assert!(b.0 - a.1 >= SIBLING_GAP - 1e-6);
    }

    /// Two roots of a forest nest into each other like two siblings.
    #[test]
    fn a_forest_packs_its_roots_by_contour() {
        let wide = (300.0, 48.0);
        let thin = (20.0, 48.0);
        // Root A is 20px wide down to row 1 and 628px wide at row 2. Root B stops at row 1.
        let a = (
            vec![node(thin), node(thin), node(wide), node(wide)],
            vec![(0, 1), (1, 2), (1, 3)],
        );
        let b = (
            vec![node(thin), node(wide), node(wide)],
            vec![(0, 1), (0, 2)],
        );
        let alone = tree(&graph(a.0.clone(), a.1.clone())).bounds.width()
            + SIBLING_GAP
            + tree(&graph(b.0.clone(), b.1.clone())).bounds.width();

        let mut nodes = a.0;
        let offset = nodes.len();
        nodes.extend(b.0);
        let mut edges = a.1;
        edges.extend(b.1.iter().map(|&(x, y)| (x + offset, y + offset)));
        let g = graph(nodes, edges);
        let l = tree(&g);
        assert!(overlapping_pairs(&g, &l).is_empty());
        assert!(
            l.bounds.width() < 0.85 * alone,
            "roots were not packed: {} vs {alone} apart",
            l.bounds.width()
        );
    }

    /// Rank is the depth, and the layout reports its row count like a layered one.
    #[test]
    fn depth_is_the_rank_and_rows_are_counted() {
        let g = graph(
            vec![node(OP), node(LEAF), node(OP), node(LEAF)],
            vec![(0, 1), (0, 2), (2, 3)],
        );
        let l = tree(&g);
        assert_eq!(
            l.nodes.iter().map(|p| p.rank).collect::<Vec<_>>(),
            vec![Some(0), Some(1), Some(1), Some(2)]
        );
        assert_eq!(l.layer_count, 3);
        // Order is the position within the depth row, in node order.
        assert_eq!(
            l.nodes.iter().map(|p| p.order).collect::<Vec<_>>(),
            vec![0, 0, 1, 0]
        );
        assert!(l.bounds.width() > 0.0 && l.bounds.height() > 0.0);
    }

    #[test]
    fn one_route_per_edge() {
        let g = graph(vec![node(OP), node(LEAF), node(LEAF)], vec![(0, 1), (0, 2)]);
        assert_eq!(tree(&g).edges.len(), 2);
    }

    /// A [`TreeEdgeStyle::Straight`] edge has two points, each on its node's border, for any
    /// shape.
    #[test]
    fn every_edge_is_clipped_to_both_borders() {
        use crate::geometry::shape::outline_distance;
        let mut g = graph(
            vec![node(OP), node(LEAF), node(LEAF), node(LEAF)],
            vec![(0, 1), (0, 2), (0, 3)],
        );
        g.nodes[0].shape = Shape::Ellipse;
        let l = layout(&g, &straight()).unwrap();
        for (i, e) in g.edges.iter().enumerate() {
            let (a, b) = (e.source.index(), e.target.index());
            let r = &l.edges[i].points;
            assert_eq!(r.len(), 2, "edge {i} is not a straight clipped line: {r:?}");
            assert_eq!(l.edges[i].kind, RouteKind::Direct);
            assert!(l.edges[i].source_port.is_some() && l.edges[i].target_port.is_some());
            for (k, node) in [(0usize, a), (1usize, b)] {
                let o = Outline {
                    center: (l.nodes[node].center.x, l.nodes[node].center.y),
                    size: (g.nodes[node].size.width, g.nodes[node].size.height),
                    shape: g.nodes[node].shape,
                };
                let d = outline_distance((r[k].x, r[k].y), o);
                assert!(d < 1e-6, "edge {i} end {k}: {d}px off node {node}'s border");
            }
            // The clipped line still points from the parent's centre at the child's.
            let dir = |p: Point, q: Point| fp::atan2(q.x - p.x, q.y - p.y);
            assert!(
                (dir(r[0], r[1]) - dir(l.nodes[a].center, l.nodes[b].center)).abs() < 1e-9,
                "edge {i} does not run centre to centre"
            );
        }
    }

    /// A 100 000-level chain lays out on a 512 KiB thread, half the wasm main stack.
    #[test]
    fn a_deep_chain_does_not_overflow_the_stack() {
        let handle = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                let n = 100_000;
                let g = graph(
                    (0..n).map(|_| node(LEAF)).collect(),
                    (0..n - 1).map(|i| (i, i + 1)).collect(),
                );
                let l = layout(&g, &TreeOptions::default()).unwrap();
                assert_eq!(l.nodes.len(), n);
                // A chain is one column, with every node on the same cross coordinate.
                assert!((l.nodes[0].center.x - l.nodes[n - 1].center.x).abs() < 1e-6);
            })
            .unwrap();
        handle.join().unwrap();
    }

    /// Two runs of the same input are identical. Nothing here reads a hash map or a clock.
    #[test]
    fn the_layout_is_deterministic() {
        let mut nodes = vec![node(OP)];
        let mut edges = Vec::new();
        for i in 0..60 {
            nodes.push(node(if i % 3 == 0 { OP } else { LEAF }));
            edges.push((i / 3, i + 1));
        }
        let g = graph(nodes, edges);
        let a = tree(&g);
        let b = tree(&g);
        assert_eq!(a, b);
    }

    #[test]
    fn a_single_leaf_lays_out() {
        let g = graph(vec![node(LEAF)], vec![]);
        assert_eq!(tree(&g).nodes.len(), 1);
    }

    #[test]
    fn an_empty_graph_lays_out() {
        assert_eq!(tree(&Graph::new()).nodes.len(), 0);
    }

    #[test]
    fn rejects_a_node_with_two_parents() {
        let g = graph(vec![node(OP), node(OP), node(LEAF)], vec![(0, 2), (1, 2)]);
        assert_eq!(
            layout(&g, &tb()).unwrap_err(),
            LayoutError::NotATree {
                node: NodeId(2),
                reason: TreeReason::MultipleParents
            }
        );
    }

    #[test]
    fn rejects_a_self_loop() {
        let g = graph(vec![node(OP), node(LEAF)], vec![(0, 1), (1, 1)]);
        assert_eq!(
            layout(&g, &tb()).unwrap_err(),
            LayoutError::NotATree {
                node: NodeId(1),
                reason: TreeReason::SelfLoop
            }
        );
    }

    #[test]
    fn rejects_a_cycle() {
        let g = graph(vec![node(OP), node(OP)], vec![(0, 1), (1, 0)]);
        assert_eq!(
            layout(&g, &tb()).unwrap_err(),
            LayoutError::NotATree {
                node: NodeId(0),
                reason: TreeReason::NoRoot
            }
        );
    }

    /// A cycle beside a valid tree still leaves a root, so only the reachability sweep catches it.
    #[test]
    fn rejects_a_cycle_beside_a_tree() {
        let g = graph(
            vec![node(OP), node(LEAF), node(OP), node(OP)],
            vec![(0, 1), (2, 3), (3, 2)],
        );
        assert_eq!(
            layout(&g, &tb()).unwrap_err(),
            LayoutError::NotATree {
                node: NodeId(2),
                reason: TreeReason::Unreachable
            }
        );
    }

    #[test]
    fn rejects_an_out_of_range_edge() {
        let g = graph(vec![node(OP)], vec![(0, 7)]);
        assert!(layout(&g, &tb()).is_err());
    }

    /// The elbow runs from the middle of the parent's bottom border, across the row gap, to the
    /// middle of the child's top border.
    #[test]
    fn an_elbow_runs_through_the_empty_row_gap() {
        let o = elbow();
        let g = graph(
            vec![node(OP), node(LEAF), node(LEAF), node(LEAF)],
            vec![(0, 1), (0, 2), (0, 3)],
        );
        let l = layout(&g, &o).unwrap();
        let gap_lo = l.nodes[0].center.y + OP.1 / 2.0;
        let gap_hi = l.nodes[1].center.y - LEAF.1 / 2.0;
        for (e, child) in l.edges.iter().zip([1usize, 2, 3]) {
            assert_eq!(e.kind, RouteKind::Direct);
            let p = &e.points;
            assert_eq!(p.len(), if child == 2 { 2 } else { 4 }, "child {child}");
            // Out of the middle of the parent's bottom border, into the middle of the child's top.
            assert!((p[0].x - l.nodes[0].center.x).abs() < 1e-6 && (p[0].y - gap_lo).abs() < 1e-6);
            let last = p[p.len() - 1];
            assert!(
                (last.x - l.nodes[child].center.x).abs() < 1e-6 && (last.y - gap_hi).abs() < 1e-6
            );
            // Every corner sits strictly inside the row gap, where no node can be.
            for mid in &p[1..p.len() - 1] {
                assert!(
                    mid.y > gap_lo && mid.y < gap_hi,
                    "{mid:?} is not in the gap"
                );
            }
        }
        // The middle child is directly below its parent, so it keeps the plain vertical line.
        assert_eq!(l.edges[1].points.len(), 2);
    }

    /// Left-to-right swaps the axes, so the elbow runs down the middle of the column gap.
    #[test]
    fn an_elbow_transposes_with_the_direction() {
        let o = TreeOptions {
            direction: Direction::LeftRight,
            ..elbow()
        };
        let g = graph(vec![node(OP), node(LEAF), node(LEAF)], vec![(0, 1), (0, 2)]);
        let l = layout(&g, &o).unwrap();
        let gap_lo = l.nodes[0].center.x + OP.0 / 2.0;
        for e in &l.edges {
            assert_eq!(e.points.len(), 4);
            assert!((e.points[0].y - l.nodes[0].center.y).abs() < 1e-6);
            assert!((e.points[0].x - gap_lo).abs() < 1e-6);
            assert!(e.points[1].x > gap_lo && (e.points[1].x - e.points[2].x).abs() < 1e-6);
        }
    }

    /// A wide fan drawn with elbows runs no edge over the boxes beside its parent, unlike
    /// straight lines.
    #[test]
    fn elbows_take_the_edges_out_of_the_nodes() {
        // A wide box beside the operator whose fan overhangs it.
        let mut nodes = vec![node(OP), node((320.0, 60.0)), node(OP)];
        let mut edges = vec![(0, 1), (0, 2)];
        for k in 3..3 + 8 {
            nodes.push(node(LEAF));
            edges.push((2, k));
        }
        let g = graph(nodes, edges);
        let hits = |o: &TreeOptions| {
            let l = layout(&g, o).unwrap();
            crate::metrics::measure(
                &g,
                &crate::LayoutOptions {
                    direction: o.direction,
                    ..crate::LayoutOptions::layered_default()
                },
                &l,
            )
            .node_edge_hits
        };
        assert!(
            hits(&straight()) > 0,
            "the straight fan was expected to graze the box beside its parent"
        );
        assert_eq!(hits(&elbow()), 0);
    }

    /// A tidy tree has forks and no joins, so the fork/join metric has nothing to say about one.
    #[test]
    fn a_tree_has_no_fork_join_blocks_to_align() {
        let g = graph(
            vec![node(OP), node(OP), node(LEAF), node(LEAF), node(LEAF)],
            vec![(0, 1), (0, 2), (1, 3), (1, 4)],
        );
        for o in [tb(), elbow()] {
            let l = layout(&g, &o).unwrap();
            let m = crate::metrics::measure(&g, &crate::LayoutOptions::layered_default(), &l);
            assert_eq!(m.split_pairs, 0);
            assert!((m.split_alignment - 1.0).abs() < 1e-9);
        }
    }
}
