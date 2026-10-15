//! Conformance cases that the core and every binding must draw identically.
//!
//! `cases/{layout,tree}/*.json` under `tests/conformance/` are requests and `expected.json` holds
//! each answer's fingerprint. The npm and Python suites check the same files. To regenerate the
//! cases and fingerprints:
//!
//! ```text
//! GRAPHLAY_BLESS=1 cargo test --release -p graphlay --all-features --test conformance
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use graphlay::{
    fixture, json, Direction, Edge, GraphBuilder, GroupId, GroupLanes, LayoutOptions, NodeId,
    PortHint, RoutingStyle, Side, Size,
};
use serde_json::{json as j, Value};

fn conformance_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/conformance")
}

/// Fixtures small enough to read as a case, by name.
const FIXTURES: &[&str] = &[
    "dfg-small-example-all",
    "dfg-small-example-all-diag",
    "dfg-an1-all",
    "pn-small-example-im",
    "ocdeclare-ocel2-p2p",
    "ocdfg-ocel2-p2p-top3types",
    "typegraph-ocel2-p2p",
    "parallel-selfloops",
    "diamond-blocks",
    "multihop-run-through-rank",
    "chain-10-lr",
    "tree-small-example-im",
    "tree-an1-im",
];

/// `(kind, name, request)` for every case: `kind` is `layout` or `tree`.
fn cases() -> Vec<(&'static str, String, Value)> {
    let mut out = Vec::new();
    let fixtures_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let fixtures = fixture::load_dir(&fixtures_dir).expect("the fixtures load");
    for name in FIXTURES {
        let f = fixtures
            .iter()
            .find(|f| f.name == *name)
            .unwrap_or_else(|| panic!("no fixture {name}"));
        let (graph, options) = f.to_graph();
        if f.options.tree {
            let request = j!({ "graph": graph, "options": f.to_tree_options() });
            out.push(("tree", f.name.clone(), request));
        } else {
            let request = j!({ "graph": graph, "preset": "layered", "options": options });
            out.push(("layout", f.name.clone(), request));
        }
    }
    for (name, graph, options) in built_cases() {
        let request = j!({ "graph": graph, "preset": "layered", "options": options });
        out.push(("layout", name.to_string(), request));
    }
    out
}

fn boxes(b: &mut GraphBuilder, n: usize) -> Vec<NodeId> {
    (0..n).map(|_| b.add_box(Size::new(100.0, 44.0))).collect()
}

/// Requests built with the API, one per feature the fixtures do not reach.
fn built_cases() -> Vec<(&'static str, graphlay::Graph, LayoutOptions)> {
    let mut out = Vec::new();

    let mut b = GraphBuilder::new();
    let n = boxes(&mut b, 2);
    b.connect(n[0], n[1]);
    out.push(("two-boxes", b.build(), LayoutOptions::layered_default()));

    let mut b = GraphBuilder::new();
    let n = boxes(&mut b, 3);
    b.connect(n[0], n[1]);
    b.connect(n[1], n[2]);
    b.connect(n[2], n[0]);
    out.push(("cycle", b.build(), LayoutOptions::layered_default()));

    let mut b = GraphBuilder::new();
    let n = boxes(&mut b, 6);
    for w in n.windows(2) {
        b.connect(w[0], w[1]);
    }
    b.connect(n[0], n[3]);
    b.connect(n[1], n[5]);
    let mut o = LayoutOptions::process_map();
    o.routing.style = RoutingStyle::Diagonal;
    out.push(("process-map-diagonal", b.build(), o));

    let mut b = GraphBuilder::new();
    let n = boxes(&mut b, 9);
    for (i, &spoke) in n[1..].iter().enumerate() {
        if i % 2 == 0 {
            b.connect(n[0], spoke);
        } else {
            b.connect(spoke, n[0]);
        }
    }
    out.push(("hub-and-spoke", b.build(), LayoutOptions::hub_and_spoke()));

    let mut b = GraphBuilder::new();
    let n = boxes(&mut b, 14);
    for w in n.windows(2) {
        b.connect(w[0], w[1]);
    }
    let mut o = LayoutOptions::process_map();
    o.direction = Direction::LeftRight;
    o.wrap.viewport_aspect = 16.0 / 9.0;
    out.push(("wrapped-chain", b.build(), o));

    let mut b = GraphBuilder::new();
    let n = boxes(&mut b, 5);
    b.connect(n[0], n[2]);
    b.connect(n[2], n[3]);
    b.connect(n[3], n[4]);
    let mut g = b.build();
    g.nodes[2].parent = Some(n[1]);
    g.nodes[3].parent = Some(n[1]);
    out.push(("compound", g, LayoutOptions::layered_default()));

    let mut b = GraphBuilder::new();
    let n = boxes(&mut b, 3);
    let mut e = Edge::new(n[0], n[1]);
    e.source_port = Some(PortHint::Offset {
        side: Side::Bottom,
        t: 0.2,
    });
    b.add_edge(e);
    let mut e = Edge::new(n[0], n[2]);
    e.source_port = Some(PortHint::Offset {
        side: Side::Bottom,
        t: 0.8,
    });
    e.target_port = Some(PortHint::Side(Side::Top));
    b.add_edge(e);
    let mut o = LayoutOptions::layered_default();
    o.direction = Direction::TopBottom;
    out.push(("port-hints", b.build(), o));

    let mut b = GraphBuilder::new();
    let n = boxes(&mut b, 3);
    b.connect(n[0], n[1]);
    b.connect(n[1], n[1]);
    b.connect(n[1], n[1]);
    b.connect(n[1], n[2]);
    out.push(("self-loops", b.build(), LayoutOptions::layered_default()));

    let mut b = GraphBuilder::new();
    let n = boxes(&mut b, 6);
    b.connect(n[0], n[1]);
    b.connect(n[1], n[2]);
    b.connect(n[0], n[3]);
    b.connect(n[3], n[4]);
    b.connect(n[4], n[5]);
    b.connect(n[2], n[5]);
    let mut g = b.build();
    for (i, node) in g.nodes.iter_mut().enumerate() {
        node.group = Some(GroupId(if i < 3 { 0 } else { 1 }));
    }
    let mut o = LayoutOptions::process_map();
    o.order.group_lanes = GroupLanes::Strict;
    out.push(("lanes", g, o));

    out
}

fn answer(kind: &str, request: &str) -> String {
    let drawn = match kind {
        "tree" => json::layout_tree(request),
        _ => json::layout(request),
    };
    match drawn {
        Ok(layout) => json::fingerprint(&layout).expect("a layout fingerprints"),
        Err(e) => format!("error: {e}"),
    }
}

#[test]
fn every_binding_reproduces_these_drawings() {
    let dir = conformance_dir();
    if std::env::var_os("GRAPHLAY_BLESS").is_some() {
        for kind in ["layout", "tree"] {
            let sub = dir.join("cases").join(kind);
            fs::remove_dir_all(&sub).ok();
            fs::create_dir_all(&sub).expect("the cases directory");
        }
        let mut expected = BTreeMap::new();
        for (kind, name, request) in cases() {
            let text = serde_json::to_string_pretty(&request).expect("a request serialises");
            fs::write(
                dir.join("cases").join(kind).join(format!("{name}.json")),
                text + "\n",
            )
            .expect("a case writes");
            expected.insert(format!("{kind}/{name}"), answer(kind, &request.to_string()));
        }
        let text = serde_json::to_string_pretty(&expected).expect("the map serialises");
        fs::write(dir.join("expected.json"), text + "\n").expect("expected.json writes");
        return;
    }

    let expected: BTreeMap<String, String> = serde_json::from_str(
        &fs::read_to_string(dir.join("expected.json")).expect("tests/conformance/expected.json"),
    )
    .expect("expected.json is a map of fingerprints");
    let mut mismatches = Vec::new();
    let mut seen = 0;
    for kind in ["layout", "tree"] {
        let sub = dir.join("cases").join(kind);
        for entry in fs::read_dir(&sub).expect("the cases directory") {
            let path = entry.expect("a directory entry").path();
            let name = path
                .file_stem()
                .expect("a file")
                .to_string_lossy()
                .to_string();
            let key = format!("{kind}/{name}");
            let request = fs::read_to_string(&path).expect("a case reads");
            let got = answer(kind, &request);
            seen += 1;
            match expected.get(&key) {
                Some(want) if *want == got => {}
                Some(want) => mismatches.push(format!("{key}: expected {want}, got {got}")),
                None => mismatches.push(format!("{key}: not in expected.json")),
            }
        }
    }
    assert_eq!(
        seen,
        expected.len(),
        "every expected fingerprint has its case file"
    );
    assert!(
        mismatches.is_empty(),
        "{} drawing(s) changed:\n  {}\nIf the change is intended, re-bless with \
         GRAPHLAY_BLESS=1 cargo test --release -p graphlay --all-features --test conformance",
        mismatches.len(),
        mismatches.join("\n  ")
    );
}
