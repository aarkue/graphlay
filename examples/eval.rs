//! Layout evaluation harness: lays out every fixture, measures the result and prints a table.
//!
//! ```text
//! cargo run -p graphlay --release --features fixtures --example eval -- --help
//! ```
//!
//! Exit code is 1 when any fixture failed to lay out (error or panic), 2 on a bad command line,
//! 0 otherwise. Fixtures load in path order and every metric is deterministic, so two runs of the
//! same code print identical tables apart from timings.

use std::collections::BTreeMap;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use graphlay::fixture::{self, Fixture};
use graphlay::geometry::{Point, Size};
use graphlay::metrics::{self, LayoutMetrics, ScoreWeights};
use graphlay::pipeline::{self, PhaseSteps};
use graphlay::svg::{
    render, sketch_scene, EdgeLabel, HeuristicMeasurer, Scene, SketchOptions, TextRun, Theme,
};
use graphlay::synthetic::synthetic_fixtures;
use graphlay::{
    tree, CrossPlacement, Direction, EdgeRoute, Graph, LabelPlacement, LabelReservation, Layout,
    LayoutOptions, Placement, PortSides, RouteKind, TreeOptions,
};

const USAGE: &str = "\
usage: cargo run -p graphlay --release --features fixtures --example eval -- [args]

  --fixtures <dir>        fixture root (default: <crate>/fixtures), read recursively
  --filter <substr>       only fixtures whose name or kind contains <substr>
  --svg-out <dir>         render one SVG per fixture into <dir>
  --layouts <dir>         do not lay anything out: read <dir>/<name>.json ({centers, routes})
                          and measure and render *that* drawing instead. Points another engine's
                          output at this crate's metrics and renderer, so a before/after
                          comparison differs only in the layout.
  --baseline <file.json>  compare against saved metrics; print deltas and mark regressions
  --save-baseline <file>  write this run's metrics as a new baseline
  --repeat <n>            lay each fixture out n times and take the median duration (default 1)
  --cycles <strategy>     minimum-reversal (default) or follow-flow
  --effort <level>        fast, balanced (default) or thorough
  --wrap <w/h>            wrap every fixture into rows for a viewport of this aspect ratio
  --wide                  print a second table with the metrics the main table leaves out
  --phase-profile         for every selected fixture, the staircase steps over the waypoint
                          chains after each pipeline phase - which phase bends a long edge and
                          which one straightens it
  --stability             also lay each layered fixture out with its lightest 10% of edges
                          removed, and report how far the drawing moved (weight 0 in the score)
  --stability-svg <dir>   with --stability, draw the full graph, the filtered one laid out fresh
                          and the filtered one with SeedMode::Structure, into <dir>/{full,fresh,
                          structure}/<name>.svg
  --port-sides <mode>     lay every fixture out with this Routing::port_sides instead of its
                          preset's: any, facing_only, facing_preferred or long_edges_any
  --merge-edges <mode>    Routing::merge_edges for every fixture: none, forks, joins or both
  --group-lanes <mode>    OrderOptions::group_lanes for every fixture: soft or strict
  --placement <name>      lay every fixture out with this CoordOptions::placement instead of
                          its preset's: brandes_koepf (bk) or network_simplex (ns)
  --face-rounds <n>       CoordTuning::simplex_face_rounds instead of its preset's; 0 leaves
                          the network simplex on whichever vertex of its optimal face it stopped
  --labels <mode>         lay every fixture out with this LabelOptions::placement instead of its
                          preset's: candidates or gap_centre
  --label-reserve <mode>  LabelOptions::reservation instead of its preset's: always, when_needed
                          or never
  --lane-budget <n>       OrderOptions::group_lane_budget_percent instead of its preset's: the
                          crossings a drawing may pay to put its groups in lanes
  --lane-area <n>         OrderOptions::group_lane_area_percent instead of its preset's: how much
                          wider the order axis may get to hold the lanes
  --gen-synthetic <dir>   (re)generate the deterministic synthetic fixtures into <dir> and exit
  --help                  print this text";

struct Args {
    fixtures: PathBuf,
    filter: Option<String>,
    svg_out: Option<PathBuf>,
    layouts: Option<PathBuf>,
    dump_layouts: Option<PathBuf>,
    step_flatten: Option<f64>,
    main_path_room: Option<u32>,
    baseline: Option<PathBuf>,
    save_baseline: Option<PathBuf>,
    repeat: usize,
    gen_synthetic: Option<PathBuf>,
    wide: bool,
    phase_profile: bool,
    stability: bool,
    stability_svg: Option<PathBuf>,
    port_sides: Option<PortSides>,
    merge_edges: Option<graphlay::MergeEdges>,
    group_lanes: Option<graphlay::GroupLanes>,
    wrap: Option<f64>,
    cycles: Option<graphlay::CycleStrategy>,
    effort: Option<graphlay::Effort>,
    placement: Option<CrossPlacement>,
    face_rounds: Option<u32>,
    flatten_room: Option<u32>,
    width_slack: Option<u32>,
    labels: Option<LabelPlacement>,
    label_reserve: Option<LabelReservation>,
    lane_budget: Option<u32>,
    lane_area: Option<u32>,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            fixtures: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures"),
            filter: None,
            svg_out: None,
            layouts: None,
            dump_layouts: None,
            step_flatten: None,
            main_path_room: None,
            baseline: None,
            save_baseline: None,
            repeat: 1,
            gen_synthetic: None,
            wide: false,
            phase_profile: false,
            stability: false,
            stability_svg: None,
            port_sides: None,
            merge_edges: None,
            group_lanes: None,
            wrap: None,
            cycles: None,
            effort: None,
            placement: None,
            face_rounds: None,
            flatten_room: None,
            width_slack: None,
            labels: None,
            label_reserve: None,
            lane_budget: None,
            lane_area: None,
        }
    }
}

fn parse_args(argv: Vec<String>) -> Result<Option<Args>, String> {
    let mut args = Args::default();
    let mut it = argv.into_iter();
    while let Some(flag) = it.next() {
        let mut value = || {
            it.next().ok_or_else(|| format!("{flag} needs a value")) // one closure per flag
        };
        match flag.as_str() {
            "--help" | "-h" => return Ok(None),
            "--fixtures" => args.fixtures = PathBuf::from(value()?),
            "--filter" => args.filter = Some(value()?),
            "--svg-out" => args.svg_out = Some(PathBuf::from(value()?)),
            "--layouts" => args.layouts = Some(PathBuf::from(value()?)),
            "--dump-layouts" => args.dump_layouts = Some(PathBuf::from(value()?)),
            "--main-path-room" => {
                let raw = value()?;
                args.main_path_room = Some(
                    raw.parse::<u32>()
                        .map_err(|e| format!("--main-path-room {raw}: {e}"))?,
                );
            }
            "--step-flatten" => {
                let raw = value()?;
                args.step_flatten = Some(
                    raw.parse::<f64>()
                        .map_err(|e| format!("--step-flatten {raw}: {e}"))?,
                );
            }
            "--baseline" => args.baseline = Some(PathBuf::from(value()?)),
            "--save-baseline" => args.save_baseline = Some(PathBuf::from(value()?)),
            "--gen-synthetic" => args.gen_synthetic = Some(PathBuf::from(value()?)),
            "--wide" => args.wide = true,
            "--cycles" => {
                let raw = value()?;
                args.cycles = Some(match raw.as_str() {
                    "minimum-reversal" => graphlay::CycleStrategy::MinimumReversal,
                    "follow-flow" => graphlay::CycleStrategy::FollowFlow,
                    _ => return Err(format!("--cycles {raw}: not a strategy")),
                });
            }
            "--effort" => {
                let raw = value()?;
                args.effort = Some(match raw.as_str() {
                    "fast" => graphlay::Effort::Fast,
                    "balanced" => graphlay::Effort::Balanced,
                    "thorough" => graphlay::Effort::Thorough,
                    _ => return Err(format!("--effort {raw}: not an effort")),
                });
            }
            "--wrap" => {
                let raw = value()?;
                args.wrap = Some(raw.parse().map_err(|e| format!("--wrap {raw}: {e}"))?);
            }
            "--phase-profile" => args.phase_profile = true,
            "--stability" => args.stability = true,
            "--stability-svg" => args.stability_svg = Some(PathBuf::from(value()?)),
            "--port-sides" => {
                let raw = value()?;
                args.port_sides = Some(match raw.as_str() {
                    "any" => PortSides::Any,
                    "facing_only" => PortSides::FacingOnly,
                    "facing_preferred" => PortSides::FacingPreferred,
                    "long_edges_any" => PortSides::LongEdgesAny,
                    _ => return Err(format!("--port-sides {raw}: not a mode")),
                });
            }
            "--group-lanes" => {
                let raw = value()?;
                args.group_lanes = Some(match raw.as_str() {
                    "soft" => graphlay::GroupLanes::Soft,
                    "strict" => graphlay::GroupLanes::Strict,
                    _ => return Err(format!("--group-lanes {raw}: not a mode")),
                });
            }
            "--merge-edges" => {
                let raw = value()?;
                args.merge_edges = Some(match raw.as_str() {
                    "none" => graphlay::MergeEdges::None,
                    "forks" => graphlay::MergeEdges::Forks,
                    "joins" => graphlay::MergeEdges::Joins,
                    "both" => graphlay::MergeEdges::Both,
                    _ => return Err(format!("--merge-edges {raw}: not a mode")),
                });
            }
            "--placement" => {
                let raw = value()?;
                args.placement = Some(match raw.as_str() {
                    "brandes_koepf" | "bk" => CrossPlacement::BrandesKoepf,
                    "network_simplex" | "ns" => CrossPlacement::NetworkSimplex,
                    _ => return Err(format!("--placement {raw}: not a placement")),
                });
            }
            "--width-slack" => {
                let raw = value()?;
                args.width_slack = Some(
                    raw.parse::<u32>()
                        .map_err(|e| format!("--width-slack {raw}: {e}"))?,
                );
            }
            "--flatten-room" => {
                let raw = value()?;
                args.flatten_room = Some(
                    raw.parse::<u32>()
                        .map_err(|e| format!("--flatten-room {raw}: {e}"))?,
                );
            }
            "--labels" => {
                let raw = value()?;
                args.labels = Some(match raw.as_str() {
                    "candidates" => LabelPlacement::Candidates,
                    "gap_centre" | "gap" => LabelPlacement::GapCentre,
                    _ => return Err(format!("--labels {raw}: not a placement")),
                });
            }
            "--label-reserve" => {
                let raw = value()?;
                args.label_reserve = Some(match raw.as_str() {
                    "always" => LabelReservation::Always,
                    "when_needed" | "needed" => LabelReservation::WhenNeeded,
                    "never" => LabelReservation::Never,
                    _ => return Err(format!("--label-reserve {raw}: not a policy")),
                });
            }
            "--face-rounds" => {
                let raw = value()?;
                args.face_rounds = Some(
                    raw.parse::<u32>()
                        .map_err(|e| format!("--face-rounds {raw}: {e}"))?,
                );
            }
            "--lane-budget" => {
                let raw = value()?;
                args.lane_budget = Some(
                    raw.parse::<u32>()
                        .map_err(|e| format!("--lane-budget {raw}: {e}"))?,
                );
            }
            "--lane-area" => {
                let raw = value()?;
                args.lane_area = Some(
                    raw.parse::<u32>()
                        .map_err(|e| format!("--lane-area {raw}: {e}"))?,
                );
            }
            "--repeat" => {
                let raw = value()?;
                args.repeat = raw
                    .parse::<usize>()
                    .map_err(|e| format!("--repeat {raw}: {e}"))?
                    .max(1);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(Some(args))
}

struct Row {
    name: String,
    kind: String,
    node_count: usize,
    edge_count: usize,
    self_loops: usize,
    unrouted_loops: usize,
    ms: f64,
    metrics: Option<LayoutMetrics>,
    error: Option<String>,
}

impl Row {
    fn score(&self, weights: &ScoreWeights) -> f64 {
        self.metrics.as_ref().map_or(0.0, |m| m.score(weights))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BaselineEntry {
    kind: String,
    ms: f64,
    score: f64,
    metrics: LayoutMetrics,
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1).collect()) {
        Ok(Some(a)) => a,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    if let Some(dir) = &args.gen_synthetic {
        return match generate_synthetic(dir) {
            Ok(count) => {
                println!("wrote {count} synthetic fixtures to {}", dir.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(2)
            }
        };
    }

    let fixtures = match fixture::load_dir(&args.fixtures) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let selected: Vec<Fixture> = fixtures
        .into_iter()
        .filter(|f| match &args.filter {
            Some(s) => f.name.contains(s.as_str()) || f.kind.contains(s.as_str()),
            None => true,
        })
        .collect();
    if selected.is_empty() {
        println!("no fixtures under {}", args.fixtures.display());
        return ExitCode::SUCCESS;
    }

    let weights = ScoreWeights::default();
    let rows = run_all(&selected, &args);

    print_table(&rows, &weights);
    if args.wide {
        print_wide_table(&rows);
    }
    print_aggregates(&rows, &weights);
    if args.phase_profile {
        print_phase_profiles(&selected);
    }
    if args.stability {
        print_stability(&selected, args.stability_svg.as_deref());
    }

    if let Some(path) = &args.baseline {
        match read_baseline(path) {
            Ok(base) => print_baseline_comparison(&rows, &base, &weights),
            Err(e) => eprintln!("baseline: {e}"),
        }
    }
    if let Some(path) = &args.save_baseline {
        if let Err(e) = write_baseline(path, &rows, &weights) {
            eprintln!("save-baseline: {e}");
            return ExitCode::from(2);
        }
        println!("\nwrote baseline to {}", path.display());
    }

    let failures = rows.iter().filter(|r| r.error.is_some()).count();
    if failures > 0 {
        eprintln!("\n{failures} fixture(s) failed");
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn generate_synthetic(dir: &Path) -> Result<usize, String> {
    let fixtures = synthetic_fixtures();
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for f in &fixtures {
        let path = dir.join(format!("{}.json", f.name));
        let json = fixture::to_json(f)?;
        fs::write(&path, format!("{json}\n")).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(fixtures.len())
}

fn run_all(fixtures: &[Fixture], args: &Args) -> Vec<Row> {
    // Panic messages would interleave with the table, and `catch_unwind` captures them anyway.
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let rows: Vec<Row> = fixtures.iter().map(|f| run_one(f, args)).collect();
    std::panic::set_hook(previous_hook);
    rows
}

fn run_one(f: &Fixture, args: &Args) -> Row {
    let (graph, mut options) = f.to_graph();
    if let Some(sides) = args.port_sides {
        options.routing.port_sides = sides;
    }
    if let Some(m) = args.group_lanes {
        options.order.group_lanes = m;
    }
    if let Some(m) = args.merge_edges {
        options.routing.merge_edges = m;
    }
    if let Some(aspect) = args.wrap {
        options.wrap.viewport_aspect = aspect;
    }
    if let Some(strategy) = args.cycles {
        options.cycles.strategy = strategy;
    }
    if let Some(effort) = args.effort {
        options.effort = effort;
    }
    if let Some(placement) = args.placement {
        options.coords.placement = placement;
    }
    if let Some(percent) = args.lane_budget {
        options.order.group_lane_budget_percent = percent;
    }
    if let Some(percent) = args.lane_area {
        options.order.group_lane_area_percent = percent;
    }
    if let Some(rounds) = args.face_rounds {
        options.tuning.coords.simplex_face_rounds = rounds;
    }
    if let Some(room) = args.flatten_room {
        options.tuning.coords.flatten_room = room;
    }
    if let Some(pct) = args.main_path_room {
        options.tuning.coords.main_path_room = pct;
    }
    if let Some(px) = args.step_flatten {
        options.tuning.routing.step_flatten = px;
    }
    if let Some(slack) = args.width_slack {
        options.tuning.coords.balance_width_slack = slack;
    }
    if let Some(placement) = args.labels {
        options.labels.placement = placement;
    }
    if let Some(reservation) = args.label_reserve {
        options.labels.reservation = reservation;
    }
    let tree_options = f.options.tree.then(|| f.to_tree_options());
    let mut row = Row {
        name: f.name.clone(),
        kind: f.kind.clone(),
        node_count: f.nodes.len(),
        edge_count: f.edges.len(),
        self_loops: f.self_loop_edges().len(),
        unrouted_loops: 0,
        ms: 0.0,
        metrics: None,
        error: None,
    };

    if let Some(dir) = &args.layouts {
        match read_dump(dir, &f.name).and_then(|d| layout_from_dump(f, &options, &d)) {
            Ok(l) => {
                row.metrics = Some(metrics::measure(&graph, &options, &l));
                if let Some(svg_dir) = &args.svg_out {
                    if let Err(e) = write_svg(svg_dir, f, &graph, &l) {
                        row.error = Some(format!("svg: {e}"));
                    }
                }
            }
            Err(e) => row.error = Some(e),
        }
        return row;
    }

    let mut timings = Vec::with_capacity(args.repeat);
    let mut layout: Option<Layout> = None;
    for _ in 0..args.repeat {
        let start = Instant::now();
        match timed_layout(&graph, &options, tree_options.as_ref()) {
            Ok(l) => {
                timings.push(start.elapsed().as_secs_f64() * 1000.0);
                layout = Some(l);
            }
            Err(e) => {
                row.error = Some(e);
                return row;
            }
        }
    }
    row.ms = median(&mut timings);
    let layout = match layout {
        Some(l) => l,
        None => return row,
    };
    row.unrouted_loops = layout
        .edges
        .iter()
        .filter(|e| e.kind == RouteKind::SelfLoop && e.points.len() < 2)
        .count();
    row.metrics = Some(metrics::measure(&graph, &options, &layout));
    if let Some(dir) = &args.dump_layouts {
        if let Err(e) = write_dump(dir, &f.name, &layout) {
            row.error = Some(format!("dump: {e}"));
        }
    }

    if let Some(dir) = &args.svg_out {
        if let Err(e) = write_svg(dir, f, &graph, &layout) {
            row.error = Some(format!("svg: {e}"));
        }
    }
    row
}

/// A drawing from another engine: node centres and edge polylines only.
#[derive(Debug, Clone, Deserialize)]
struct LayoutDump {
    centers: Vec<[f64; 2]>,
    /// One polyline per fixture edge. Empty means the engine did not route that edge.
    routes: Vec<Vec<[f64; 2]>>,
}

/// Writes a drawing in the `{centers, routes}` shape that `--layouts` reads.
fn write_dump(dir: &Path, name: &str, layout: &Layout) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let centers: Vec<[f64; 2]> = layout
        .nodes
        .iter()
        .map(|p| [p.center.x, p.center.y])
        .collect();
    let routes: Vec<Vec<[f64; 2]>> = layout
        .edges
        .iter()
        .map(|e| e.points.iter().map(|p| [p.x, p.y]).collect())
        .collect();
    let json = serde_json::json!({ "centers": centers, "routes": routes });
    let path = dir.join(format!("{name}.json"));
    fs::write(&path, json.to_string()).map_err(|e| format!("{}: {e}", path.display()))
}

fn read_dump(dir: &Path, name: &str) -> Result<LayoutDump, String> {
    let path = dir.join(format!("{name}.json"));
    let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

/// Rebuilds a `Layout` from `dump`'s geometry, with sizes from the fixture. Rank and order are
/// read back off the drawing so metrics that use them mean the same for both engines.
fn layout_from_dump(
    f: &Fixture,
    options: &LayoutOptions,
    dump: &LayoutDump,
) -> Result<Layout, String> {
    if dump.centers.len() != f.nodes.len() {
        return Err(format!(
            "dump has {} centres for {} nodes",
            dump.centers.len(),
            f.nodes.len()
        ));
    }
    if dump.routes.len() != f.edges.len() {
        return Err(format!(
            "dump has {} routes for {} edges",
            dump.routes.len(),
            f.edges.len()
        ));
    }
    let along_y = options.direction == Direction::TopBottom;
    let axis = |p: &[f64; 2]| if along_y { p[1] } else { p[0] };
    let cross = |p: &[f64; 2]| if along_y { p[0] } else { p[1] };

    // A gap wider than half the median node extent starts a new layer.
    let mut sorted: Vec<(usize, f64)> = dump
        .centers
        .iter()
        .enumerate()
        .map(|(i, c)| (i, axis(c)))
        .collect();
    sorted.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut extents: Vec<f64> = f
        .nodes
        .iter()
        .map(|n| if along_y { n.height } else { n.width })
        .collect();
    let tolerance = median(&mut extents) / 2.0;
    let mut rank_of = vec![0i32; dump.centers.len()];
    let mut rank = 0i32;
    for w in 0..sorted.len() {
        if w > 0 && sorted[w].1 - sorted[w - 1].1 > tolerance {
            rank += 1;
        }
        rank_of[sorted[w].0] = rank;
    }
    let mut by_rank: BTreeMap<i32, Vec<usize>> = BTreeMap::new();
    for (i, r) in rank_of.iter().enumerate() {
        by_rank.entry(*r).or_default().push(i);
    }
    let mut order_of = vec![0u32; dump.centers.len()];
    for members in by_rank.values_mut() {
        members.sort_by(|a, b| {
            cross(&dump.centers[*a])
                .partial_cmp(&cross(&dump.centers[*b]))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for (slot, i) in members.iter().enumerate() {
            order_of[*i] = slot as u32;
        }
    }

    let nodes: Vec<Placement> = dump
        .centers
        .iter()
        .enumerate()
        .map(|(i, c)| Placement {
            center: Point::new(c[0], c[1]),
            size: Size::new(f.nodes[i].width, f.nodes[i].height),
            rank: Some(rank_of[i]),
            order: order_of[i],
        })
        .collect();

    let edges: Vec<EdgeRoute> = dump
        .routes
        .iter()
        .enumerate()
        .map(|(i, pts)| {
            let (from, to) = (f.edges[i].from, f.edges[i].to);
            let points: Vec<Point> = pts.iter().map(|p| Point::new(p[0], p[1])).collect();
            let spans = (rank_of[to] - rank_of[from]).unsigned_abs();
            let kind = if from == to {
                RouteKind::SelfLoop
            } else if spans > 1 {
                RouteKind::Chained { spans }
            } else {
                RouteKind::Direct
            };
            EdgeRoute::routed(points, kind, &nodes[from], &nodes[to], None)
        })
        .collect();

    let layer_count = by_rank.len() as u32;
    let mut layout = Layout {
        nodes,
        edges,
        bounds: Default::default(),
        layer_count,
        reversed: Vec::new(),
        stats: Default::default(),
        junctions: Vec::new(),
        lanes: Vec::new(),
    };
    layout.recompute_bounds();
    Ok(layout)
}

fn timed_layout(
    graph: &Graph,
    options: &LayoutOptions,
    tree_options: Option<&TreeOptions>,
) -> Result<Layout, String> {
    let run = || match tree_options {
        Some(t) => tree::layout(graph, t),
        None => graphlay::layout(graph, options),
    };
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(Ok(layout)) => Ok(layout),
        Ok(Err(e)) => Err(format!("error: {e}")),
        Err(payload) => Err(format!("PANIC: {}", panic_message(&payload))),
    }
}

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    }
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    values[values.len() / 2]
}

fn print_table(rows: &[Row], weights: &ScoreWeights) {
    let name_w = rows.iter().map(|r| r.name.len()).max().unwrap_or(4).max(4);
    let kind_w = rows.iter().map(|r| r.kind.len()).max().unwrap_or(4).max(4);
    // `fit` is the font size once scaled into 1600x900 (below about 7 labels are unreadable),
    // `split` is aligned/total fork-join blocks, `main` the share of the heaviest path on one line
    // and `flow` the share of drawn length heading the majority way. Width stays within 170.
    println!(
        "{:<name_w$}  {:<kind_w$}  {:>4}  {:>5}  {:>6}  {:>6}  {:>6}  {:>5}  {:>4}  {:>5}  {:>7}  {:>6}  {:>10}  {:>4}  {:>4}  {:>4}  {:>7}  {:>8}",
        "name", "kind", "n", "e", "ms", "cross", "wcross", "hits", "lbl", "bends", "area(k)",
        "elen", "w x h", "fit", "main", "flow", "split", "score",
    );
    for r in rows {
        match (&r.metrics, &r.error) {
            (Some(m), None) => println!(
                "{:<name_w$}  {:<kind_w$}  {:>4}  {:>5}  {:>6.2}  {:>6}  {:>6.0}  {:>5}  {:>4}  {:>5}  {:>7.1}  {:>6.0}  {:>10}  {:>4.1}  {:>4.2}  {:>4.2}  {:>7}  {:>8.1}",
                r.name,
                r.kind,
                r.node_count,
                r.edge_count,
                r.ms,
                m.crossings,
                m.weighted_crossings,
                m.node_edge_hits,
                m.label_overlaps,
                m.bends,
                m.area / 1000.0,
                m.total_edge_length,
                format!("{:.0}x{:.0}", m.width, m.height),
                m.fit_font_px,
                m.main_path_aligned_fraction,
                m.flow_consistency,
                format!("{}/{}", m.split_aligned, m.split_pairs),
                m.score(weights),
            ),
            _ => println!(
                "{:<name_w$}  {:<kind_w$}  {:>4}  {:>5}  {:>6}  {}",
                r.name,
                r.kind,
                r.node_count,
                r.edge_count,
                "-",
                r.error.as_deref().unwrap_or("no layout"),
            ),
        }
    }

    let mut defects: Vec<String> = Vec::new();
    for r in rows {
        let Some(m) = &r.metrics else { continue };
        if m.node_overlaps > 0 {
            defects.push(format!(
                "  {}: {} overlapping node pair(s)",
                r.name, m.node_overlaps
            ));
        }
        if m.overlaps > 0 {
            defects.push(format!(
                "  {}: {} collinear edge overlap(s)",
                r.name, m.overlaps
            ));
        }
        if m.label_overlaps > 0 {
            defects.push(format!(
                "  {}: {} label collision(s) of {} reserved label(s) ({} label/label, {} label/node, {} label/edge)",
                r.name,
                m.label_overlaps,
                m.label_count,
                m.label_label_overlaps,
                m.label_node_overlaps,
                m.label_edge_hits
            ));
        }
        if m.endpoint_off_outline > 0 {
            defects.push(format!(
                "  {}: {} route endpoint(s) off their node outline (worst {:.1}px)",
                r.name,
                m.endpoint_off_outline,
                m.max_endpoint_outline_gap.unwrap_or(0.0)
            ));
        }
        if let Some(gap) = m.min_node_gap {
            if gap < 0.0 {
                defects.push(format!(
                    "  {}: nodes interpenetrate by {:.1}px",
                    r.name, -gap
                ));
            }
        }
    }
    if !defects.is_empty() {
        println!("\ndefects:");
        for line in defects {
            println!("{line}");
        }
    }

    // Unrouted self-loops are reported separately below.
    let unrouted: Vec<(&Row, usize)> = rows
        .iter()
        .filter_map(|r| {
            let n = r
                .metrics
                .as_ref()
                .map_or(0, |m| m.unrouted_edges.saturating_sub(r.unrouted_loops));
            (n > 0).then_some((r, n))
        })
        .collect();
    if !unrouted.is_empty() {
        println!("\nedges without a polyline (edge metrics do not apply to them):");
        for (r, n) in unrouted {
            println!("  {}: {n} of {} edge(s)", r.name, r.edge_count);
        }
    }

    let with_loops: Vec<&Row> = rows.iter().filter(|r| r.unrouted_loops > 0).collect();
    if !with_loops.is_empty() {
        println!(
            "\nself-loops the engine did not route (emitted as a pointless RouteKind::SelfLoop):"
        );
        for r in with_loops {
            println!(
                "  {}: {} of {} self-loop edge(s)",
                r.name, r.unrouted_loops, r.self_loops
            );
        }
    }
}

/// The quality metrics without a column in the main table.
fn print_wide_table(rows: &[Row]) {
    println!("\nevery other metric (--wide):");
    let name_w = rows.iter().map(|r| r.name.len()).max().unwrap_or(4).max(4);
    println!(
        "{:<name_w$}  {:>4}  {:>11}  {:>6}  {:>8}  {:>9}  {:>5}  {:>8}  {:>5}  {:>5}  {:>5}  {:>5}  {:>6}  {:>5}  {:>5}  {:>6}  {:>5}  {:>5}  {:>5}  {:>5}",
        "name", "lbls", "l/l l/n l/e", "lclr", "tcross", "ang min/mn", "<30d", "parallel",
        "block", "lane", "intr", "wsbal", "turn", "tmax", "mlen", "mdev", "steps", "rever", "stair",
        "pends",
    );
    for r in rows {
        let Some(m) = &r.metrics else { continue };
        println!(
            "{:<name_w$}  {:>4}  {:>11}  {:>6}  {:>8.0}  {:>9}  {:>5}  {:>8.0}  {:>5}  {:>5.2}  {:>5}  {:>5.2}  {:>6.1}  {:>5.1}  {:>5}  {:>6.3}  {:>5}  {:>5}  {:>5}  {:>5}",
            r.name,
            m.label_count,
            format!(
                "{} {} {}",
                m.label_label_overlaps, m.label_node_overlaps, m.label_edge_hits
            ),
            m.min_label_clearance
                .map_or_else(|| "-".to_string(), |v| format!("{v:.1}")),
            m.thickness_weighted_crossings,
            format!(
                "{}/{}",
                m.crossing_angle_min
                    .map_or_else(|| "-".to_string(), |v| format!("{v:.0}")),
                m.crossing_angle_mean
                    .map_or_else(|| "-".to_string(), |v| format!("{v:.0}"))
            ),
            m.crossings_below_30deg,
            m.parallel_run_length,
            m.block_intrusion,
            m.lane_purity,
            m.lane_intruders,
            m.whitespace_balance,
            m.path_turn_mean,
            m.path_turn_max,
            m.main_path_len,
            m.main_path_deviation,
            m.short_steps,
            m.route_reversals,
            m.staircase_steps,
            m.perpendicular_ends,
        );
    }
}

/// Fraction of a fixture's edges dropped for the stability run.
const STABILITY_DROP: f64 = 0.10;

/// Staircase steps on the waypoint chains after each phase, with the same definition as the
/// `stair` column of `--wide`. Phases that change nothing still get a row.
fn print_phase_profiles(fixtures: &[Fixture]) {
    println!("\nstaircase steps over the waypoint chains, after each pipeline phase:");
    let layered: Vec<&Fixture> = fixtures.iter().filter(|f| !f.options.tree).collect();
    if layered.is_empty() {
        println!("  (no layered fixture selected)");
        return;
    }
    for f in layered {
        let (graph, options) = f.to_graph();
        let rows = match catch_unwind(AssertUnwindSafe(|| {
            pipeline::phase_profile(&graph, &options)
        })) {
            Ok(Ok(rows)) => rows,
            Ok(Err(e)) => {
                println!("  {}: error: {e}", f.name);
                continue;
            }
            Err(payload) => {
                println!("  {}: PANIC: {}", f.name, panic_message(&payload));
                continue;
            }
        };
        let Some(last) = rows.last() else {
            continue;
        };
        // When the real layout rebuilds the graph, these rows describe only one run of it.
        let note = match pipeline::front_end(&graph, &options) {
            Some(what) => format!(", one run of the phases, not {what}"),
            None => String::new(),
        };
        println!(
            "  {} ({} chains, {} of them level at the end{note}):",
            f.name, last.chains, last.level
        );
        let mut previous: Option<PhaseSteps> = None;
        for r in &rows {
            let delta = previous.map_or(0i64, |p: PhaseSteps| r.steps as i64 - p.steps as i64);
            println!(
                "    {:<20} steps {:>4} ({:>+4})  jogs {:>5}  level {:>4}",
                r.phase, r.steps, delta, r.jogs, r.level
            );
            previous = Some(*r);
        }
    }
}

/// Lays each layered fixture out whole and with its lightest edges removed, as a process-mining
/// frequency filter would, and reports how far the drawing moved. Tidy-tree fixtures are skipped.
fn print_stability(fixtures: &[Fixture], svg_dir: Option<&Path>) {
    println!(
        "\nstability under dropping the lightest {:.0}% of edges (weight 0 in the score):",
        STABILITY_DROP * 100.0
    );
    let layered: Vec<&Fixture> = fixtures.iter().filter(|f| !f.options.tree).collect();
    let name_w = layered
        .iter()
        .map(|f| f.name.len())
        .max()
        .unwrap_or(4)
        .max(4);
    println!(
        "{:<name_w$}  {:>5}  | {:>22} | {:>22} | {:>22} | {:>16}",
        "", "", "fresh", "seeded", "structure kept", "nodes fixed"
    );
    println!(
        "{:<name_w$}  {:>5}  | {:>6} {:>5} {:>5} {:>4} | {:>6} {:>5} {:>5} {:>4} | {:>6} {:>5} {:>5} {:>4} | {:>5} {:>5} {:>4}",
        "name", "drop", "hausd", "ortho", "cross", "bend", "hausd", "ortho", "cross", "bend",
        "hausd", "ortho", "cross", "bend", "cross", "bend", "hits"
    );
    let mut totals = [0usize; 5];
    let mut ortho_sum = [0.0f64; 3];
    let mut rows = 0usize;
    for f in layered {
        let (graph, options) = f.to_graph();
        let Ok(before) = graphlay::layout(&graph, &options) else {
            println!("{:<name_w$}  (did not lay out)", f.name);
            continue;
        };
        let (filtered, dropped) = drop_lightest(&graph, STABILITY_DROP);
        let Ok(after) = graphlay::layout(&filtered, &options) else {
            println!("{:<name_w$}  (filtered graph did not lay out)", f.name);
            continue;
        };
        let common: Vec<(graphlay::NodeId, graphlay::NodeId)> = (0..graph.nodes.len() as u32)
            .map(|i| (graphlay::NodeId(i), graphlay::NodeId(i)))
            .collect();
        // Seeded from the full drawing, once as placement seeds only and once keeping structure.
        let mut seeded_graph = filtered.clone();
        for (node, placed) in seeded_graph.nodes.iter_mut().zip(&before.nodes) {
            node.position = Some(placed.center);
        }
        let mut keep = options;
        keep.seed_mode = graphlay::SeedMode::Structure;
        let (Ok(seeded), Ok(kept)) = (
            graphlay::layout(&seeded_graph, &options),
            graphlay::layout(&seeded_graph, &keep),
        ) else {
            println!("{:<name_w$}  (seeded layout failed)", f.name);
            continue;
        };
        if let Some(dir) = svg_dir {
            for (sub, g, l) in [
                ("full", &graph, &before),
                ("fresh", &filtered, &after),
                ("structure", &filtered, &kept),
            ] {
                if let Err(e) = write_svg(&dir.join(sub), f, g, l) {
                    eprintln!("{e}");
                }
            }
        }
        let rerouted = graphlay::reroute(&seeded_graph, &options)
            .ok()
            .map(|l| metrics::measure(&filtered, &options, &l));
        let (s, ss, sk) = (
            metrics::compare(&before, &after, &common),
            metrics::compare(&before, &seeded, &common),
            metrics::compare(&before, &kept, &common),
        );
        let (mf, ms, mk) = (
            metrics::measure(&filtered, &options, &after),
            metrics::measure(&filtered, &options, &seeded),
            metrics::measure(&filtered, &options, &kept),
        );
        let (rc, rb, rh) = rerouted
            .as_ref()
            .map_or((0, 0, 0), |m| (m.crossings, m.bends, m.node_edge_hits));
        println!(
            "{:<name_w$}  {:>5}  | {:>6.0} {:>5.2} {:>5} {:>4} | {:>6.0} {:>5.2} {:>5} {:>4} | {:>6.0} {:>5.2} {:>5} {:>4} | {:>5} {:>5} {:>4}",
            f.name,
            dropped,
            s.hausdorff,
            s.orthogonal_ordering,
            mf.crossings,
            mf.bends,
            ss.hausdorff,
            ss.orthogonal_ordering,
            ms.crossings,
            ms.bends,
            sk.hausdorff,
            sk.orthogonal_ordering,
            mk.crossings,
            mk.bends,
            rc,
            rb,
            rh,
        );
        for (t, v) in totals
            .iter_mut()
            .zip([mf.crossings, ms.crossings, mk.crossings, rc, rh])
        {
            *t += v;
        }
        for (t, v) in ortho_sum.iter_mut().zip([
            s.orthogonal_ordering,
            ss.orthogonal_ordering,
            sk.orthogonal_ordering,
        ]) {
            *t += v;
        }
        rows += 1;
    }
    let n = rows.max(1) as f64;
    println!(
        "TOTAL crossings fresh {} seeded {} structure {} fixed {} (fixed hits {}); mean ortho fresh {:.3} seeded {:.3} structure {:.3}",
        totals[0], totals[1], totals[2], totals[3], totals[4],
        ortho_sum[0] / n, ortho_sum[1] / n, ortho_sum[2] / n
    );
}

/// `graph` without the lightest `fraction` of its edges, by `(weight, index)` with self-loops
/// last, and how many were removed.
fn drop_lightest(graph: &Graph, fraction: f64) -> (Graph, usize) {
    let mut order: Vec<usize> = (0..graph.edges.len()).collect();
    order.sort_unstable_by(|&a, &b| {
        let (ea, eb) = (&graph.edges[a], &graph.edges[b]);
        ea.weight.total_cmp(&eb.weight).then(a.cmp(&b))
    });
    let drop_count = ((graph.edges.len() as f64 * fraction).floor() as usize)
        .min(graph.edges.len().saturating_sub(1));
    let mut keep = vec![true; graph.edges.len()];
    for &i in order.iter().take(drop_count) {
        keep[i] = false;
    }
    let edges = graph
        .edges
        .iter()
        .enumerate()
        .filter(|(i, _)| keep[*i])
        .map(|(_, e)| e.clone())
        .collect();
    (
        Graph {
            nodes: graph.nodes.clone(),
            edges,
        },
        drop_count,
    )
}

fn print_aggregates(rows: &[Row], weights: &ScoreWeights) {
    println!("\nper kind:");
    let mut kinds: BTreeMap<&str, Vec<&Row>> = BTreeMap::new();
    for r in rows {
        kinds.entry(r.kind.as_str()).or_default().push(r);
    }
    println!(
        "{:<14}  {:>5}  {:>6}  {:>6}  {:>5}  {:>6}  {:>10}  {:>10}",
        "kind", "count", "failed", "cross", "hits", "bends", "mean score", "total ms"
    );
    for (kind, group) in &kinds {
        print_aggregate_line(kind, group, weights);
    }
    let all: Vec<&Row> = rows.iter().collect();
    print_aggregate_line("ALL", &all, weights);
}

fn print_aggregate_line(label: &str, group: &[&Row], weights: &ScoreWeights) {
    let failed = group.iter().filter(|r| r.error.is_some()).count();
    let ok: Vec<&&Row> = group.iter().filter(|r| r.metrics.is_some()).collect();
    let sum = |f: fn(&LayoutMetrics) -> usize| -> usize {
        ok.iter().filter_map(|r| r.metrics.as_ref()).map(f).sum()
    };
    let mean_score = if ok.is_empty() {
        0.0
    } else {
        ok.iter().map(|r| r.score(weights)).sum::<f64>() / ok.len() as f64
    };
    let total_ms: f64 = group.iter().map(|r| r.ms).sum();
    println!(
        "{:<14}  {:>5}  {:>6}  {:>6}  {:>5}  {:>6}  {:>10.1}  {:>10.2}",
        label,
        group.len(),
        failed,
        sum(|m| m.crossings),
        sum(|m| m.node_edge_hits),
        sum(|m| m.bends),
        mean_score,
        total_ms,
    );
}

fn read_baseline(path: &Path) -> Result<BTreeMap<String, BaselineEntry>, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn write_baseline(path: &Path, rows: &[Row], weights: &ScoreWeights) -> Result<(), String> {
    let mut out: BTreeMap<String, BaselineEntry> = BTreeMap::new();
    for r in rows {
        if let Some(m) = &r.metrics {
            out.insert(
                r.name.clone(),
                BaselineEntry {
                    kind: r.kind.clone(),
                    ms: r.ms,
                    score: m.score(weights),
                    metrics: m.clone(),
                },
            );
        }
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
    }
    let text = serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?;
    fs::write(path, format!("{text}\n")).map_err(|e| format!("{}: {e}", path.display()))
}

fn print_baseline_comparison(
    rows: &[Row],
    base: &BTreeMap<String, BaselineEntry>,
    weights: &ScoreWeights,
) {
    println!("\nvs baseline (delta = now - baseline; positive is worse except ms):");
    let name_w = rows.iter().map(|r| r.name.len()).max().unwrap_or(4).max(4);
    println!(
        "{:<name_w$}  {:>7}  {:>7}  {:>7}  {:>7}  {:>11}  {:>9}  {:>9}  verdict",
        "name", "dcross", "dhits", "dbends", "dxbend", "darea(kpx2)", "dscore", "dms"
    );
    let (mut improved, mut regressed, mut unchanged, mut missing) = (0, 0, 0, 0);
    let mut new_terms_only = 0;
    let mut lost = 0;
    // The score without the label, main-path and flow terms, for baselines that predate them.
    let legacy = ScoreWeights {
        label_overlap: 0.0,
        main_path: 0.0,
        flow: 0.0,
        ..weights.clone()
    };
    for r in rows {
        let Some(entry) = base.get(&r.name) else {
            missing += 1;
            println!("{:<name_w$}  {:>7}  (new fixture, no baseline)", r.name, "");
            continue;
        };
        let Some(m) = &r.metrics else {
            regressed += 1;
            println!(
                "{:<name_w$}  {}",
                r.name,
                r.error.as_deref().unwrap_or("no layout")
            );
            continue;
        };
        let d_score = m.score(weights) - entry.score;
        if (m.score(&legacy) - entry.score).abs() <= 1e-6 && d_score > 1e-6 {
            new_terms_only += 1;
        }
        // An unrouted edge has no crossings and a missing label no collisions, so a drop in
        // coverage counts as a regression whatever the score says.
        let lost_coverage = m.unrouted_edges > entry.metrics.unrouted_edges
            || m.label_count < entry.metrics.label_count;
        let verdict = if lost_coverage {
            regressed += 1;
            lost += 1;
            "LOST COVERAGE"
        } else if d_score > 1e-6 {
            regressed += 1;
            "REGRESSED"
        } else if d_score < -1e-6 {
            improved += 1;
            "improved"
        } else {
            unchanged += 1;
            "unchanged"
        };
        println!(
            "{:<name_w$}  {:>+7}  {:>+7}  {:>+7}  {:>+7}  {:>+11.1}  {:>+9.1}  {:>+9.2}  {}",
            r.name,
            m.crossings as i64 - entry.metrics.crossings as i64,
            m.node_edge_hits as i64 - entry.metrics.node_edge_hits as i64,
            m.bends as i64 - entry.metrics.bends as i64,
            m.excess_bends as i64 - entry.metrics.excess_bends as i64,
            (m.area - entry.metrics.area) / 1000.0,
            d_score,
            r.ms - entry.ms,
            verdict,
        );
    }
    println!(
        "\n{improved} improved, {regressed} regressed, {unchanged} unchanged, {missing} not in baseline"
    );
    if lost > 0 {
        println!(
            "of the regressions, {lost} drew less than the baseline did (unrouted edges or labels)"
        );
    }
    print_kind_medians(rows, base);
    if new_terms_only > 0 {
        println!(
            "of those, {new_terms_only} moved only because the score formula grew: the \
             pre-existing terms come out identical to the baseline's"
        );
    }
}

/// Per model kind, the median ratio `(now + 1) / (baseline + 1)` on the headline metrics. Corpus
/// sums are dominated by the largest graphs, so the median shows a typical drawing instead.
fn print_kind_medians(rows: &[Row], base: &BTreeMap<String, BaselineEntry>) {
    type Pick = fn(&LayoutMetrics) -> f64;
    let columns: [(&str, Pick); 9] = [
        ("cross", |m| m.crossings as f64),
        ("hits", |m| m.node_edge_hits as f64),
        ("xbend", |m| m.excess_bends as f64),
        ("perp", |m| m.perpendicular_ends as f64),
        ("offc", |m| m.off_centre_ends as f64),
        ("inout", |m| m.backward_nonloop_edges as f64),
        ("area", |m| m.area),
        ("elen", |m| m.total_edge_length),
        ("fit", |m| m.fit_font_px),
    ];
    let mut kinds: BTreeMap<&str, Vec<(&LayoutMetrics, &LayoutMetrics)>> = BTreeMap::new();
    for r in rows {
        if let (Some(m), Some(entry)) = (&r.metrics, base.get(&r.name)) {
            kinds
                .entry(r.kind.as_str())
                .or_default()
                .push((m, &entry.metrics));
        }
    }
    println!("\nmedian change per kind (now / baseline; below 1.00 is better except fit):");
    print!("{:<12} {:>3}", "kind", "n");
    for (name, _) in &columns {
        print!("  {name:>6}");
    }
    println!();
    for (kind, pairs) in &kinds {
        print!("{kind:<12} {:>3}", pairs.len());
        for (_, pick) in &columns {
            let mut ratios: Vec<f64> = pairs
                .iter()
                .map(|(now, was)| (pick(now) + 1.0) / (pick(was) + 1.0))
                .collect();
            print!("  {:>6.2}", median(&mut ratios));
        }
        println!();
    }
}

fn write_svg(dir: &Path, f: &Fixture, graph: &Graph, layout: &Layout) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let svg = render(
        &scene(f, graph, layout),
        &Theme::default(),
        &HeuristicMeasurer::new(),
    );
    let path = dir.join(format!("{}.svg", sanitize(&f.name)));
    fs::write(&path, svg).map_err(|e| format!("{}: {e}", path.display()))
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn scene(f: &Fixture, graph: &Graph, layout: &Layout) -> Scene {
    let groups = layout.lanes.iter().filter_map(|l| l.group).map(|g| g.0 + 1);
    let options = SketchOptions {
        labels: f.nodes.iter().map(|n| n.label.clone()).collect(),
        lane_names: (0..groups.max().unwrap_or(0))
            .map(|g| format!("group {g}"))
            .collect(),
        padding: 36.0,
        ..SketchOptions::default()
    };
    let mut scene = sketch_scene(graph, layout, &options);
    // The sketch skips unrouted edges, so its edges line up with the routed ones in order.
    let routed = layout
        .edges
        .iter()
        .enumerate()
        .filter(|(_, r)| r.points.len() >= 2);
    for ((i, route), edge) in routed.zip(&mut scene.edges) {
        let Some(text) = f.edges.get(i).and_then(|e| e.label.clone()) else {
            continue;
        };
        if text.is_empty() {
            continue;
        }
        // A chip, since a halo lets edges show through between letters.
        let label = EdgeLabel::new(TextRun::new(text).size(12.0), route.label_anchor).chip();
        edge.labels.push(label);
    }
    scene
}
