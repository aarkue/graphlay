//! The `graphlay` command: reads a JSON request and writes JSON or SVG.
//!
//! Every subcommand wraps one function of `graphlay::json`, so the command, the wasm package
//! and the Python package give the same answers.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use serde_json::Value;

#[derive(Parser)]
#[command(
    name = "graphlay",
    version,
    about = "Deterministic graph layout: JSON in, JSON or SVG out"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Lay out a graph with the layered engine
    Layout(RequestArgs),
    /// Re-route the edges of a graph over the positions its nodes carry
    Reroute(RequestArgs),
    /// Lay out a rooted tree or forest with the tidy-tree engine
    Tree(RequestArgs),
    /// Measure a layout against the request that produced it
    Metrics {
        /// The request the layout came from
        #[arg(long, value_name = "FILE")]
        request: PathBuf,
        /// The layout as `layout` printed it (`-` or omitted: stdin)
        layout: Option<PathBuf>,
        #[command(flatten)]
        out: OutputArgs,
    },
    /// Draw a layout of a request's graph as a plain SVG sketch
    Sketch {
        /// The request the layout came from
        #[arg(long, value_name = "FILE")]
        request: PathBuf,
        /// The layout as `layout` printed it (`-` or omitted: stdin)
        layout: Option<PathBuf>,
        /// Sketch options as JSON text or `@file`: labels, lane names, background, padding
        #[arg(short = 'O', long, value_name = "JSON")]
        options: Option<String>,
        /// Print the sketch as a scene (JSON) to restyle and pass to `svg`
        #[arg(long)]
        scene: bool,
        /// Write here instead of stdout
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
    /// Print the fingerprint of a layout
    Fingerprint {
        /// The layout as `layout` printed it (`-` or omitted: stdin)
        layout: Option<PathBuf>,
    },
    /// Draw a scene to SVG
    Svg {
        /// The scene (`-` or omitted: stdin)
        scene: Option<PathBuf>,
        /// The palette to draw with instead of the default one
        #[arg(long, value_name = "FILE")]
        palette: Option<PathBuf>,
        /// Write here instead of stdout
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
    /// List the presets a request can start from
    Presets,
    /// Print the full options of a preset
    Options {
        #[arg(default_value = "layered")]
        preset: String,
        #[command(flatten)]
        out: OutputArgs,
    },
    /// Print the JSON schemas of every input and output type
    Schema {
        #[command(flatten)]
        out: OutputArgs,
    },
}

#[derive(Args)]
struct RequestArgs {
    /// The request `{"graph": ..., "preset": ..., "options": ...}`, or a bare graph
    /// `{"nodes": ..., "edges": ...}` (`-` or omitted: stdin)
    request: Option<PathBuf>,
    /// Start from this preset instead of the request's
    #[arg(short, long)]
    preset: Option<String>,
    /// Options to patch over the preset, as JSON text or `@file`
    #[arg(short = 'O', long, value_name = "JSON")]
    options: Option<String>,
    /// Print a plain SVG sketch of the drawing instead of the layout
    #[arg(long)]
    sketch: bool,
    #[command(flatten)]
    out: OutputArgs,
}

#[derive(Args)]
struct OutputArgs {
    /// Write here instead of stdout
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,
    /// Indent the JSON
    #[arg(long)]
    pretty: bool,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("graphlay: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::Layout(args) => drawn(&args, graphlay::json::layout),
        Command::Reroute(args) => drawn(&args, graphlay::json::reroute),
        Command::Tree(args) => drawn(&args, graphlay::json::layout_tree),
        Command::Sketch {
            request,
            layout,
            options,
            scene,
            output,
        } => {
            let request = read(Some(&request))?;
            let layout = read(layout.as_deref())?;
            let options = match options {
                Some(o) => inline_or_file(&o)?,
                None => String::new(),
            };
            let draw = if scene {
                graphlay::json::sketch_scene
            } else {
                graphlay::json::sketch
            };
            emit(output.as_deref(), &draw(&request, &layout, &options)?)
        }
        Command::Metrics {
            request,
            layout,
            out,
        } => {
            let request = read(Some(&request))?;
            let layout = read(layout.as_deref())?;
            emit_json(&out, &graphlay::json::metrics(&request, &layout)?)
        }
        Command::Fingerprint { layout } => {
            let layout = read(layout.as_deref())?;
            emit(
                None,
                &format!("{}\n", graphlay::json::fingerprint(&layout)?),
            )
        }
        Command::Svg {
            scene,
            palette,
            output,
        } => {
            let scene = read(scene.as_deref())?;
            let palette = match palette {
                Some(p) => read(Some(&p))?,
                None => String::new(),
            };
            emit(
                output.as_deref(),
                &graphlay::json::render_scene(&scene, &palette)?,
            )
        }
        Command::Presets => {
            let mut text = graphlay::json::presets().join("\n");
            text.push('\n');
            emit(None, &text)
        }
        Command::Options { preset, out } => emit_json(&out, &graphlay::json::options(&preset)?),
        Command::Schema { out } => emit_json(&out, &graphlay::json::schemas()),
    }
}

/// Runs one of the drawing entry points on the request and prints the layout, or its sketch.
fn drawn(args: &RequestArgs, draw: fn(&str) -> Result<String, String>) -> Result<(), String> {
    let request = request(args)?;
    let out = draw(&request)?;
    if args.sketch {
        emit(
            args.out.output.as_deref(),
            &graphlay::json::sketch(&request, &out, "")?,
        )
    } else {
        emit_json(&args.out, &out)
    }
}

/// JSON given inline, or in the file an `@` prefix names.
fn inline_or_file(text: &str) -> Result<String, String> {
    match text.strip_prefix('@') {
        Some(path) => read(Some(Path::new(path))),
        None => Ok(text.to_string()),
    }
}

/// The request text, with a bare graph wrapped and the `--preset` and `--options` flags applied.
fn request(args: &RequestArgs) -> Result<String, String> {
    let mut v: Value = serde_json::from_str(&read(args.request.as_deref())?)
        .map_err(|e| format!("request: {e}"))?;
    if !v.is_object() {
        return Err("request: expected a JSON object".into());
    }
    if v.get("graph").is_none() && v.get("nodes").is_some() {
        v = serde_json::json!({ "graph": v });
    }
    if let Some(preset) = &args.preset {
        v["preset"] = Value::String(preset.clone());
    }
    if let Some(options) = &args.options {
        let text = inline_or_file(options)?;
        let patch: Value = serde_json::from_str(&text).map_err(|e| format!("options: {e}"))?;
        merge(&mut v["options"], &patch);
    }
    Ok(v.to_string())
}

fn merge(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(b), Value::Object(p)) => {
            for (k, v) in p {
                merge(b.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (b, p) => *b = p.clone(),
    }
}

/// The whole of a file, or of stdin for `-` and for no path.
fn read(path: Option<&Path>) -> Result<String, String> {
    match path {
        Some(p) if p != Path::new("-") => {
            fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))
        }
        _ => {
            let mut text = String::new();
            io::stdin()
                .read_to_string(&mut text)
                .map_err(|e| format!("stdin: {e}"))?;
            Ok(text)
        }
    }
}

fn emit_json(out: &OutputArgs, json: &str) -> Result<(), String> {
    let mut text = if out.pretty {
        let v: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?
    } else {
        json.to_string()
    };
    text.push('\n');
    emit(out.output.as_deref(), &text)
}

fn emit(path: Option<&Path>, text: &str) -> Result<(), String> {
    match path {
        Some(p) => fs::write(p, text).map_err(|e| format!("{}: {e}", p.display())),
        None => io::stdout()
            .write_all(text.as_bytes())
            .map_err(|e| format!("stdout: {e}")),
    }
}
