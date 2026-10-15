//! The engine for JavaScript: one function per entry point of `graphlay::json`, JSON text in
//! and JSON text out. The npm package in `js/` wraps these with types and a loader.

use wasm_bindgen::prelude::*;

/// The engine's version.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Lays out the graph of a layout request. See `graphlay::json::layout`.
#[wasm_bindgen]
pub fn layout(request: &str) -> Result<String, String> {
    graphlay::json::layout(request)
}

/// Re-routes the edges of a layout request over the positions its nodes carry.
#[wasm_bindgen]
pub fn reroute(request: &str) -> Result<String, String> {
    graphlay::json::reroute(request)
}

/// Lays out the tree of a tree request.
#[wasm_bindgen]
pub fn layout_tree(request: &str) -> Result<String, String> {
    graphlay::json::layout_tree(request)
}

/// Measures a layout against the request that produced it.
#[wasm_bindgen]
pub fn metrics(request: &str, layout: &str) -> Result<String, String> {
    graphlay::json::metrics(request, layout)
}

/// The fingerprint of a layout.
#[wasm_bindgen]
pub fn fingerprint(layout: &str) -> Result<String, String> {
    graphlay::json::fingerprint(layout)
}

/// The preset names, as a JSON array.
#[wasm_bindgen]
pub fn presets() -> String {
    let names = graphlay::json::presets();
    serde_json_array(&names)
}

/// The full options of a preset, as JSON.
#[wasm_bindgen]
pub fn options(preset: &str) -> Result<String, String> {
    graphlay::json::options(preset)
}

/// Draws a scene to SVG markup. An empty palette means the default one.
#[cfg(feature = "svg")]
#[wasm_bindgen]
pub fn render_scene(scene: &str, palette: &str) -> Result<String, String> {
    graphlay::json::render_scene(scene, palette)
}

/// Draws a layout of the request's graph as a plain sketch. Empty options mean the defaults.
#[cfg(feature = "svg")]
#[wasm_bindgen]
pub fn sketch(request: &str, layout: &str, options: &str) -> Result<String, String> {
    graphlay::json::sketch(request, layout, options)
}

/// The plain sketch as a scene, to restyle before `render_scene`.
#[cfg(feature = "svg")]
#[wasm_bindgen]
pub fn sketch_scene(request: &str, layout: &str, options: &str) -> Result<String, String> {
    graphlay::json::sketch_scene(request, layout, options)
}

/// A JSON array of strings, without pulling serde_json into the binary for one call.
fn serde_json_array(items: &[String]) -> String {
    let quoted: Vec<String> = items
        .iter()
        .map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect();
    format!("[{}]", quoted.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REQUEST: &str = r#"{"graph": {"nodes": [{"size": {"width": 80, "height": 40}},
        {"size": {"width": 80, "height": 40}}], "edges": [{"source": 0, "target": 1}]}}"#;

    #[test]
    fn the_entry_points_answer_with_json() {
        let out = layout(REQUEST).unwrap();
        assert!(out.starts_with('{'));
        assert_eq!(fingerprint(&out).unwrap().len(), 16);
        assert!(metrics(REQUEST, &out).unwrap().contains("crossings"));
        assert_eq!(
            presets(),
            r#"["layered","process_map","hub_and_spoke","hub_and_spoke_flow"]"#
        );
        assert!(options("layered").unwrap().contains("spacing"));
        assert!(options("curvy").unwrap_err().contains("unknown preset"));
        assert!(layout("{").unwrap_err().contains("EOF"));
    }
}
