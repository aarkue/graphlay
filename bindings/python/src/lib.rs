//! The engine for Python: `graphlay._native`, one function per entry point of
//! `graphlay::json`, JSON text in and JSON text out. The package's `__init__.py` turns dicts,
//! graphs from networkx and JSON into requests and wraps the answers.

use pyo3::prelude::*;

pyo3::create_exception!(
    graphlay,
    GraphlayError,
    pyo3::exceptions::PyValueError,
    "An error the engine reported: a bad request, an invalid graph or a limit exceeded."
);

fn fail(message: String) -> PyErr {
    GraphlayError::new_err(message)
}

/// The engine's version.
#[pyfunction]
fn version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Lays out the graph of a layout request.
#[pyfunction]
fn layout(request: &str) -> PyResult<String> {
    graphlay::json::layout(request).map_err(fail)
}

/// Re-routes the edges of a layout request over the positions its nodes carry.
#[pyfunction]
fn reroute(request: &str) -> PyResult<String> {
    graphlay::json::reroute(request).map_err(fail)
}

/// Lays out the tree of a tree request.
#[pyfunction]
fn layout_tree(request: &str) -> PyResult<String> {
    graphlay::json::layout_tree(request).map_err(fail)
}

/// Measures a layout against the request that produced it.
#[pyfunction]
fn metrics(request: &str, layout: &str) -> PyResult<String> {
    graphlay::json::metrics(request, layout).map_err(fail)
}

/// The fingerprint of a layout.
#[pyfunction]
fn fingerprint(layout: &str) -> PyResult<String> {
    graphlay::json::fingerprint(layout).map_err(fail)
}

/// The preset names.
#[pyfunction]
fn presets() -> Vec<String> {
    graphlay::json::presets()
}

/// The full options of a preset, as JSON.
#[pyfunction]
fn options(preset: &str) -> PyResult<String> {
    graphlay::json::options(preset).map_err(fail)
}

/// Draws a scene to SVG markup. An empty palette means the default one.
#[pyfunction]
fn render_scene(scene: &str, palette: &str) -> PyResult<String> {
    graphlay::json::render_scene(scene, palette).map_err(fail)
}

/// Draws a layout of the request's graph as a plain sketch. Empty options mean the defaults.
#[pyfunction]
fn sketch(request: &str, layout: &str, options: &str) -> PyResult<String> {
    graphlay::json::sketch(request, layout, options).map_err(fail)
}

/// The plain sketch as a scene, to restyle before `render_scene`.
#[pyfunction]
fn sketch_scene(request: &str, layout: &str, options: &str) -> PyResult<String> {
    graphlay::json::sketch_scene(request, layout, options).map_err(fail)
}

#[pymodule]
fn _native(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("GraphlayError", m.py().get_type::<GraphlayError>())?;
    m.add_function(wrap_pyfunction!(version, m)?)?;
    m.add_function(wrap_pyfunction!(layout, m)?)?;
    m.add_function(wrap_pyfunction!(reroute, m)?)?;
    m.add_function(wrap_pyfunction!(layout_tree, m)?)?;
    m.add_function(wrap_pyfunction!(metrics, m)?)?;
    m.add_function(wrap_pyfunction!(fingerprint, m)?)?;
    m.add_function(wrap_pyfunction!(presets, m)?)?;
    m.add_function(wrap_pyfunction!(options, m)?)?;
    m.add_function(wrap_pyfunction!(render_scene, m)?)?;
    m.add_function(wrap_pyfunction!(sketch, m)?)?;
    m.add_function(wrap_pyfunction!(sketch_scene, m)?)?;
    Ok(())
}
