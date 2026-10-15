//! The command as a user runs it: every subcommand once, over a pipe.

use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::Value;

const GRAPH: &str = r#"{"nodes": [{"size": {"width": 80, "height": 40}},
    {"size": {"width": 80, "height": 40}}], "edges": [{"source": 0, "target": 1}]}"#;

/// Runs `graphlay` with `args` and `stdin`: `(succeeded, stdout, stderr)`.
fn graphlay(args: &[&str], stdin: &str) -> (bool, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_graphlay"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary runs");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(stdin.as_bytes())
        .expect("stdin accepts the request");
    let out = child.wait_with_output().expect("the binary exits");
    (
        out.status.success(),
        String::from_utf8(out.stdout).expect("utf-8 stdout"),
        String::from_utf8(out.stderr).expect("utf-8 stderr"),
    )
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).expect("JSON output")
}

#[test]
fn a_bare_graph_on_stdin_is_laid_out() {
    let (ok, out, err) = graphlay(&["layout", "--preset", "process_map", "--pretty"], GRAPH);
    assert!(ok, "{err}");
    let v = json(&out);
    assert_eq!(v["nodes"].as_array().map(Vec::len), Some(2));
    assert!(out.contains("\n  "), "pretty output is indented");
    // Top to bottom is the process-map preset's direction.
    assert!(v["nodes"][0]["center"]["y"].as_f64() < v["nodes"][1]["center"]["y"].as_f64());
}

#[test]
fn options_patch_the_preset_and_an_error_names_the_field() {
    let (ok, out, err) = graphlay(&["layout", "-O", r#"{"direction": "topbottom"}"#], GRAPH);
    assert!(ok, "{err}");
    let v = json(&out);
    assert!(v["nodes"][0]["center"]["y"].as_f64() < v["nodes"][1]["center"]["y"].as_f64());

    let (ok, _, err) = graphlay(
        &["layout", "-O", r#"{"routing": {"style": "curvy"}}"#],
        GRAPH,
    );
    assert!(!ok);
    assert!(err.starts_with("graphlay: options.routing.style"), "{err}");
}

#[test]
fn fingerprint_metrics_and_reroute_take_a_layout() {
    let (_, layout, _) = graphlay(&["layout"], GRAPH);
    let (ok, fp, _) = graphlay(&["fingerprint"], &layout);
    assert!(ok);
    assert_eq!(fp.trim().len(), 16);

    let request = std::env::temp_dir().join(format!("graphlay-cli-{}.json", std::process::id()));
    std::fs::write(&request, format!(r#"{{"graph": {GRAPH}}}"#)).expect("temp file");
    let (ok, metrics, err) = graphlay(
        &[
            "metrics",
            "--request",
            request.to_str().expect("utf-8 path"),
        ],
        &layout,
    );
    std::fs::remove_file(&request).ok();
    assert!(ok, "{err}");
    assert_eq!(json(&metrics)["crossings"], 0);

    let mut seeded = json(GRAPH);
    let drawn = json(&layout);
    for i in 0..2 {
        seeded["nodes"][i]["position"] = drawn["nodes"][i]["center"].clone();
    }
    let (ok, out, err) = graphlay(&["reroute"], &seeded.to_string());
    assert!(ok, "{err}");
    assert_eq!(
        json(&out)["nodes"][1]["center"],
        drawn["nodes"][1]["center"]
    );
}

#[test]
fn presets_options_and_schema_print() {
    let (ok, out, _) = graphlay(&["presets"], "");
    assert!(ok);
    assert_eq!(out.lines().count(), 4);
    let (ok, out, _) = graphlay(&["options", "hub_and_spoke"], "");
    assert!(ok);
    assert!(json(&out).get("spacing").is_some());
    let (ok, _, err) = graphlay(&["options", "curvy"], "");
    assert!(!ok);
    assert!(err.contains("unknown preset"));
    let (ok, out, _) = graphlay(&["schema"], "");
    assert!(ok);
    assert!(json(&out).get("LayoutRequest").is_some());
}

#[test]
fn svg_draws_a_scene_and_tree_uses_the_tree_engine() {
    let (ok, out, err) = graphlay(&["svg"], r#"{"nodes": [], "edges": []}"#);
    assert!(ok, "{err}");
    assert!(out.contains("<svg"));
    let (ok, out, err) = graphlay(&["tree"], GRAPH);
    assert!(ok, "{err}");
    assert_eq!(json(&out)["nodes"].as_array().map(Vec::len), Some(2));
}

#[test]
fn a_sketch_is_svg_with_the_keys_as_labels() {
    let graph = GRAPH.replacen("{\"size\"", "{\"key\": \"start\", \"size\"", 1);
    let (ok, out, err) = graphlay(&["layout", "--sketch"], &graph);
    assert!(ok, "{err}");
    assert!(out.contains("<svg") && out.contains("start"));

    let (_, layout, _) = graphlay(&["layout"], &graph);
    let request = std::env::temp_dir().join(format!("graphlay-sketch-{}.json", std::process::id()));
    std::fs::write(&request, format!(r#"{{"graph": {graph}}}"#)).expect("temp file");
    let (ok, out, err) = graphlay(
        &[
            "sketch",
            "--request",
            request.to_str().expect("utf-8 path"),
            "-O",
            r#"{"labels": ["Alpha", "Beta"]}"#,
        ],
        &layout,
    );
    std::fs::remove_file(&request).ok();
    assert!(ok, "{err}");
    assert!(out.contains("Alpha") && !out.contains("start"));
}

#[test]
fn a_missing_file_is_an_error() {
    let (ok, _, err) = graphlay(&["layout", "/nonexistent/request.json"], "");
    assert!(!ok);
    assert!(err.contains("nonexistent"));
}
