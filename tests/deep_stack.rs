//! Lays out a path graph on a 512 KiB stack, so any per-node recursion in the engine aborts here.
//! wasm gives a layout run about 1 MB of stack and a worker thread often less.

use graphlay::{layout, Direction, Graph, GraphBuilder, LayoutOptions, Size};

/// 20 000 nodes in release or with `GRAPHLAY_STRESS=1`, else 4 000, since routing is quadratic in
/// the node count. Both overflow any recursive phase.
fn path_len() -> usize {
    if cfg!(debug_assertions) && std::env::var_os("GRAPHLAY_STRESS").is_none() {
        4_000
    } else {
        20_000
    }
}

/// A path of `n` boxes: `0 -> 1 -> ... -> n-1`.
fn path_graph(n: usize) -> Graph {
    let mut b = GraphBuilder::new();
    let ids: Vec<_> = (0..n).map(|_| b.add_box(Size::new(120.0, 52.0))).collect();
    for pair in ids.windows(2) {
        b.connect(pair[0], pair[1]);
    }
    b.build()
}

#[test]
fn a_deep_path_lays_out_on_a_512_kib_stack() {
    let n = path_len();
    let o = LayoutOptions {
        direction: Direction::TopBottom,
        ..LayoutOptions::layered_default()
    };
    let laid_out = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || layout(&path_graph(n), &o))
        .expect("spawn")
        .join()
        .unwrap_or_else(|_| panic!("laying out a {n}-node path overflowed a 512 KiB stack"))
        .expect("a path graph must lay out");
    assert_eq!(laid_out.nodes.len(), n);
    assert_eq!(laid_out.edges.len(), n - 1);
    assert!(laid_out
        .nodes
        .iter()
        .all(|p| p.center.x.is_finite() && p.center.y.is_finite()));
    // One node per rank in flow order, so the drawing moves monotonically down.
    for i in 1..n {
        assert!(
            laid_out.nodes[i].center.y > laid_out.nodes[i - 1].center.y,
            "node {i} did not advance past {}",
            i - 1
        );
    }
}
