//! Writes the benchmark corpus: the plain fixtures as layout requests, for `bench/run.mjs`.
//!
//! ```text
//! cargo run --release --example bench_corpus --features fixtures -- bench/corpus
//! ```
//!
//! A fixture is left out when the other engines cannot take it as it is: a tree, a graph with
//! lanes, or one with edges ranked against their direction.

use std::fs;
use std::path::PathBuf;

use graphlay::fixture;
use serde_json::json;

fn main() {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "bench/corpus".into()),
    );
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let fixtures = fixture::load_dir(&dir).expect("the fixtures load");
    fs::create_dir_all(&out).expect("the corpus directory");
    let mut written = 0;
    for f in &fixtures {
        let plain = !f.options.tree
            && f.nodes.iter().all(|n| n.category.is_none())
            && f.edges.iter().all(|e| e.category.is_none() && !e.reversed);
        if !plain {
            continue;
        }
        let (graph, options) = f.to_graph();
        let request = json!({ "graph": graph, "preset": "layered", "options": options });
        fs::write(out.join(format!("{}.json", f.name)), request.to_string())
            .expect("a request writes");
        written += 1;
    }
    eprintln!("wrote {written} requests to {}", out.display());
}
