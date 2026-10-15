//! The gallery: every example in `tests/gallery/*.rs`, drawn into `docs/gallery/*.svg`, with
//! `docs/gallery.md` showing each drawing next to the code that made it.
//!
//! The committed drawings must match what the code draws. To regenerate them and the golden test:
//!
//! ```text
//! GRAPHLAY_BLESS=1 cargo test --release -p graphlay --all-features --test gallery --test golden
//! ```
//!
//! To add an example, write `tests/gallery/<name>.rs` (a `//!` title line, a blank `//!` line, a
//! description, and `pub fn panels() -> Vec<Panel>`), add it to [`EXAMPLES`], and bless.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use graphlay::svg::{sketch, HeuristicMeasurer, Palette, Scene, SketchOptions, Theme};
use graphlay::{Graph, Layout, LayoutOptions};

#[path = "gallery/compounds.rs"]
mod compounds;
#[path = "gallery/merge_edges.rs"]
mod merge_edges;
#[path = "gallery/port_hints.rs"]
mod port_hints;
#[path = "gallery/presets.rs"]
mod presets;
#[path = "gallery/process_models.rs"]
mod process_models;
#[path = "gallery/quick_start.rs"]
mod quick_start;
#[path = "gallery/styles.rs"]
mod styles;
#[path = "gallery/swimlanes.rs"]
mod swimlanes;
#[path = "gallery/tree.rs"]
mod tree;
#[path = "gallery/wrapping.rs"]
mod wrapping;

/// One drawing of an example: a caption, the graph, and its layout.
pub struct Panel {
    caption: String,
    graph: Graph,
    options: LayoutOptions,
    layout: Layout,
    /// Lane names by group id, for a drawing with swimlanes.
    lanes: &'static [&'static str],
    /// Node labels. `None` labels every node with its `Node::key`.
    labels: Option<Vec<String>>,
    /// A restyled drawing of the layout and the palette to draw it with, instead of the sketch.
    scene: Option<(Scene, Palette)>,
}

impl Panel {
    pub fn new(caption: &str, graph: Graph, options: &LayoutOptions, layout: Layout) -> Panel {
        Panel {
            caption: caption.to_string(),
            graph,
            options: *options,
            layout,
            lanes: &[],
            labels: None,
            scene: None,
        }
    }

    /// Draw this panel from a scene of your own instead of the plain sketch.
    pub fn scene(self, scene: Scene, palette: Palette) -> Panel {
        Panel {
            scene: Some((scene, palette)),
            ..self
        }
    }

    pub fn with_lanes(self, lanes: &'static [&'static str]) -> Panel {
        Panel { lanes, ..self }
    }
}

/// One code example: file stem, the function that draws it, its source.
type Example = (&'static str, fn() -> Vec<Panel>, &'static str);

/// The code examples, in gallery order.
const EXAMPLES: &[Example] = &[
    (
        "quick_start",
        quick_start::panels,
        include_str!("gallery/quick_start.rs"),
    ),
    (
        "process_models",
        process_models::panels,
        include_str!("gallery/process_models.rs"),
    ),
    (
        "presets",
        presets::panels,
        include_str!("gallery/presets.rs"),
    ),
    (
        "merge_edges",
        merge_edges::panels,
        include_str!("gallery/merge_edges.rs"),
    ),
    (
        "port_hints",
        port_hints::panels,
        include_str!("gallery/port_hints.rs"),
    ),
    (
        "swimlanes",
        swimlanes::panels,
        include_str!("gallery/swimlanes.rs"),
    ),
    (
        "compounds",
        compounds::panels,
        include_str!("gallery/compounds.rs"),
    ),
    (
        "wrapping",
        wrapping::panels,
        include_str!("gallery/wrapping.rs"),
    ),
    ("tree", tree::panels, include_str!("gallery/tree.rs")),
    ("styles", styles::panels, include_str!("gallery/styles.rs")),
];

/// Draws a panel as a plain sketch, or with its own style if it has one.
fn render(p: &Panel) -> String {
    if let Some((scene, palette)) = &p.scene {
        let theme = Theme::new(palette.clone());
        let svg = graphlay::svg::render(scene, &theme, &HeuristicMeasurer::new());
        // The npm and Python packages pass scenes as JSON, so each one must survive the trip.
        let text = serde_json::to_string(scene).expect("a scene serializes");
        let back: Scene = serde_json::from_str(&text).expect("a scene deserializes");
        let again = graphlay::svg::render(&back, &theme, &HeuristicMeasurer::new());
        assert_eq!(svg, again, "{} changes through JSON", p.caption);
        return svg;
    }
    let options = SketchOptions {
        labels: p.labels.clone().unwrap_or_default(),
        lane_names: p.lanes.iter().map(|s| s.to_string()).collect(),
        ..SketchOptions::default()
    };
    sketch(&p.graph, &p.layout, &options)
}

/// `(title, description)` from a source file's leading `//!` lines, and the source without them.
fn split_source(source: &str) -> (String, String, String) {
    let mut doc: Vec<&str> = vec![];
    let mut rest = source;
    while let Some(line) = rest.lines().next() {
        let Some(text) = line.strip_prefix("//!") else {
            break;
        };
        doc.push(text.strip_prefix(' ').unwrap_or(text));
        rest = &rest[line.len()..];
        rest = rest.strip_prefix('\n').unwrap_or(rest);
    }
    let title = doc.first().copied().unwrap_or_default().to_string();
    let description = doc
        .iter()
        .skip(1)
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    (title, description, rest.trim_start().to_string())
}

/// Every file the gallery consists of, by path relative to the crate root.
fn generate() -> Vec<(PathBuf, String)> {
    let mut files: Vec<(PathBuf, String)> = vec![];
    let mut md = String::new();
    md.push_str(
        "<!-- Generated by tests/gallery.rs. Do not edit by hand: change the examples in \
         tests/gallery/ and regenerate with\n     \
         GRAPHLAY_BLESS=1 cargo test --release -p graphlay --all-features --test gallery -->\n\n",
    );
    md.push_str("# Gallery\n\n");
    md.push_str(
        "Every drawing here is laid out by graphlay and drawn by its SVG renderer, from the code \
         shown under it. `tests/gallery.rs` redraws them and fails when a drawing changes. \
         `Panel` in the code is the gallery's own wrapper for a caption, the graph and its \
         layout.\n\n",
    );
    md.push_str("## Contents\n\n");
    for (_, _, source) in EXAMPLES {
        let (title, _, _) = split_source(source);
        let _ = writeln!(md, "- [{title}](#{})", anchor(&title));
    }
    md.push('\n');

    for (stem, panels, source) in EXAMPLES {
        let (title, description, code) = split_source(source);
        let panels = panels();
        let _ = writeln!(md, "## {title}\n\n{description}\n");
        for (k, panel) in panels.iter().enumerate() {
            let file = if panels.len() == 1 {
                format!("{stem}.svg")
            } else {
                format!("{stem}-{}.svg", k + 1)
            };
            let _ = writeln!(
                md,
                "**{}**\n\n![{title}: {}](gallery/{file})\n",
                panel.caption, panel.caption
            );
            files.push((Path::new("docs/gallery").join(&file), render(panel)));
        }
        let _ = writeln!(
            md,
            "<details><summary>Code (<code>tests/gallery/{stem}.rs</code>)</summary>\n\n```rust\n{}\n```\n\n</details>\n",
            code.trim_end()
        );
    }

    files.push((PathBuf::from("docs/gallery.md"), md));
    files
}

/// GitHub's heading anchor: lower case, spaces to dashes, punctuation dropped.
fn anchor(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

/// Round every number to one decimal, since the last digits of a coordinate differ between
/// platforms.
fn normalise(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut num = String::new();
    let flush = |num: &mut String, out: &mut String| {
        if num.is_empty() {
            return;
        }
        match num.parse::<f64>() {
            Ok(v) if num.contains('.') => {
                let r = (v * 10.0).round() / 10.0;
                let r = if r == 0.0 { 0.0 } else { r };
                let _ = write!(out, "{r:.1}");
            }
            _ => out.push_str(num),
        }
        num.clear();
    };
    for c in text.chars() {
        if c.is_ascii_digit() || c == '.' || (c == '-' && num.is_empty()) {
            num.push(c);
        } else {
            flush(&mut num, &mut out);
            out.push(c);
        }
    }
    flush(&mut num, &mut out);
    out
}

#[test]
fn gallery_is_up_to_date() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let files = generate();
    if std::env::var_os("GRAPHLAY_BLESS").is_some() {
        let dir = root.join("docs/gallery");
        // Remove drawings of deleted examples.
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let keep = files.iter().any(|(p, _)| root.join(p) == path);
                if !keep && path.extension().is_some_and(|e| e == "svg") {
                    std::fs::remove_file(&path).expect("remove a stale drawing");
                }
            }
        }
        std::fs::create_dir_all(&dir).expect("create docs/gallery");
        for (path, text) in &files {
            std::fs::write(root.join(path), text).expect("write the gallery");
        }
        return;
    }
    let stale: Vec<String> = files
        .iter()
        .filter(|(path, text)| {
            std::fs::read_to_string(root.join(path))
                .map_or(true, |old| normalise(&old) != normalise(text))
        })
        .map(|(path, _)| path.display().to_string())
        .collect();
    assert!(
        stale.is_empty(),
        "the gallery is out of date: {}\nregenerate it with\n  \
         GRAPHLAY_BLESS=1 cargo test --release -p graphlay --all-features --test gallery",
        stale.join(", ")
    );
}

#[test]
fn normalise_rounds_coordinates_only() {
    assert_eq!(
        normalise(r#"<path d="M1.234 -5.06 L7 8.95"/> v0.2"#),
        r#"<path d="M1.2 -5.1 L7 9.0"/> v0.2"#
    );
}

/// No example drawing has a defect the metrics can see.
#[test]
fn gallery_examples_have_no_defects() {
    use graphlay::metrics::measure;
    for (stem, panels, _) in EXAMPLES {
        for p in panels() {
            let m = measure(&p.graph, &p.options, &p.layout);
            let defects = [
                ("unrouted edges", m.unrouted_edges),
                ("overlapping nodes", m.node_overlaps),
                ("edges through nodes", m.node_edge_hits),
                ("overlapping edge segments", m.overlaps),
                ("edge ends off their node", m.endpoint_off_outline),
                ("short steps", m.short_steps),
                ("ends that turn back across their border", m.inward_ends),
                ("label overlaps", m.label_overlaps),
            ];
            for (what, count) in defects {
                assert_eq!(count, 0, "{stem} / {}: {count} {what}", p.caption);
            }
        }
    }
}

/// Prints the defect metrics of every gallery panel. Run with `--ignored --nocapture`.
#[test]
#[ignore]
fn gallery_metrics_probe() {
    use graphlay::metrics::measure;
    for (stem, panels, _) in EXAMPLES {
        for p in panels() {
            let m = measure(&p.graph, &p.options, &p.layout);
            println!(
                "{stem:12} {:28} steps={} stair={} hits={} off_outline={} overlaps={} cross={} bends={} min_port_gap={:?} pends={} offcentre={}",
                p.caption, m.short_steps, m.staircase_steps, m.node_edge_hits, m.endpoint_off_outline,
                m.overlaps, m.crossings, m.bends, m.min_port_gap.map(|g| (g * 10.0).round() / 10.0), m.perpendicular_ends, m.off_centre_ends
            );
        }
    }
}
