// Lays out the corpus with graphlay, dagre and ELK, measures every result with graphlay's own
// metrics, and writes docs/benchmarks.md. Run `pnpm corpus` first to write the requests from the
// fixtures, then `pnpm bench` here.
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { performance } from "node:perf_hooks";

import dagre from "@dagrejs/dagre";
import ELK from "elkjs/lib/elk.bundled.js";
import { load } from "graphlay";

const require = createRequire(import.meta.url);
const versions = {
  graphlay: null,
  dagre: require("@dagrejs/dagre/package.json").version,
  elk: require("elkjs/package.json").version,
};

const engine = await load();
versions.graphlay = engine.version;
const elk = new ELK();

/** The layout wire format from node centres and edge polylines, so graphlay can measure it. */
function toLayout(request, centers, routes, layerCount) {
  const nodes = request.graph.nodes.map((n, i) => ({
    center: centers[i],
    size: n.size,
    rank: 0,
    order: 0,
  }));
  const edges = request.graph.edges.map((e, i) => ({
    kind: e.source === e.target ? "selfloop" : "direct",
    points: routes[i],
  }));
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const n of nodes) {
    minX = Math.min(minX, n.center.x - n.size.width / 2);
    maxX = Math.max(maxX, n.center.x + n.size.width / 2);
    minY = Math.min(minY, n.center.y - n.size.height / 2);
    maxY = Math.max(maxY, n.center.y + n.size.height / 2);
  }
  for (const e of edges) {
    for (const p of e.points) {
      minX = Math.min(minX, p.x);
      maxX = Math.max(maxX, p.x);
      minY = Math.min(minY, p.y);
      maxY = Math.max(maxY, p.y);
    }
  }
  return {
    nodes,
    edges,
    bounds: { min: { x: minX, y: minY }, max: { x: maxX, y: maxY } },
    layer_count: layerCount,
    reversed: [],
    stats: { dummy_count: 0 },
  };
}

function spacing(request) {
  const o = request.options ?? {};
  return {
    topBottom: o.direction === "topbottom",
    nodeGap: o.spacing?.node_gap ?? 30,
    layerGap: o.spacing?.layer_gap ?? 54,
  };
}

function runDagre(request) {
  const { topBottom, nodeGap, layerGap } = spacing(request);
  const g = new dagre.graphlib.Graph({ multigraph: true })
    .setGraph({ rankdir: topBottom ? "TB" : "LR", nodesep: nodeGap, ranksep: layerGap, edgesep: 10 })
    .setDefaultEdgeLabel(() => ({}));
  request.graph.nodes.forEach((n, i) => g.setNode(String(i), { width: n.size.width, height: n.size.height }));
  request.graph.edges.forEach((e, i) =>
    g.setEdge(String(e.source), String(e.target), { weight: Math.max(1, Math.round(e.weight ?? 1)) }, `e${i}`),
  );
  dagre.layout(g);
  const centers = request.graph.nodes.map((_, i) => {
    const n = g.node(String(i));
    return { x: n.x, y: n.y };
  });
  const routes = request.graph.edges.map((e, i) => {
    const points = g.edge(String(e.source), String(e.target), `e${i}`).points;
    return points.map((p) => ({ x: p.x, y: p.y }));
  });
  const ranks = new Set(request.graph.nodes.map((_, i) => g.node(String(i)).rank));
  return toLayout(request, centers, routes, ranks.size);
}

async function runElk(request) {
  const { topBottom, nodeGap, layerGap } = spacing(request);
  const graph = {
    id: "root",
    layoutOptions: {
      "elk.algorithm": "layered",
      "elk.direction": topBottom ? "DOWN" : "RIGHT",
      "elk.spacing.nodeNode": String(nodeGap),
      "elk.layered.spacing.nodeNodeBetweenLayers": String(layerGap),
      "elk.edgeRouting": "ORTHOGONAL",
    },
    children: request.graph.nodes.map((n, i) => ({ id: `n${i}`, width: n.size.width, height: n.size.height })),
    edges: request.graph.edges.map((e, i) => ({ id: `e${i}`, sources: [`n${e.source}`], targets: [`n${e.target}`] })),
  };
  const out = await elk.layout(graph);
  const centers = out.children.map((c) => ({ x: c.x + c.width / 2, y: c.y + c.height / 2 }));
  const routes = out.edges.map((e) => {
    const s = e.sections?.[0];
    if (!s) return [];
    return [s.startPoint, ...(s.bendPoints ?? []), s.endPoint].map((p) => ({ x: p.x, y: p.y }));
  });
  return toLayout(request, centers, routes, 0);
}

/** Median wall time of `runs` calls, and the last result. */
async function timed(f, runs) {
  const times = [];
  let out;
  for (let i = 0; i < runs; i++) {
    const t0 = performance.now();
    out = await f();
    times.push(performance.now() - t0);
  }
  times.sort((a, b) => a - b);
  return { ms: times[Math.floor(times.length / 2)], out };
}

/** The request with `effort: "fast"`, the setting for an interactive relayout. */
function fast(request) {
  return { ...request, options: { ...(request.options ?? {}), effort: "fast" } };
}

const engines = {
  graphlay: (request) => engine.layout(request),
  "graphlay fast": (request) => engine.layout(fast(request)),
  dagre: runDagre,
  elk: runElk,
};

const files = readdirSync("corpus")
  .filter((f) => f.endsWith(".json"))
  .map((f) => ({ name: f.replace(/\.json$/, ""), request: JSON.parse(readFileSync(`corpus/${f}`, "utf8")) }))
  .sort((a, b) => a.request.graph.nodes.length - b.request.graph.nodes.length || a.name.localeCompare(b.name));

const rows = [];
for (const { name, request } of files) {
  const n = request.graph.nodes.length;
  const runs = n > 500 ? 2 : n > 100 ? 3 : 5;
  const row = { name, nodes: n, edges: request.graph.edges.length, results: {} };
  for (const [label, run] of Object.entries(engines)) {
    try {
      await run(request); // warm up
      const { ms, out } = await timed(() => run(request), runs);
      const m = engine.metrics(label === "graphlay fast" ? fast(request) : request, out);
      row.results[label] = {
        ms,
        crossings: m.crossings,
        bends: m.bends,
        hits: m.node_edge_hits,
        length: m.total_edge_length,
        area: m.width * m.height,
      };
    } catch (e) {
      row.results[label] = { error: e instanceof Error ? e.message : String(e) };
    }
  }
  rows.push(row);
  console.log(name, JSON.stringify(row.results));
}

// The report.
const metrics = [
  ["ms", "time (ms)", (v) => (v < 10 ? v.toFixed(1) : Math.round(v).toString())],
  ["crossings", "crossings", (v) => String(v)],
  ["bends", "bends", (v) => String(v)],
  ["hits", "edges through nodes", (v) => String(v)],
  ["length", "edge length (px)", (v) => Math.round(v).toLocaleString("en-US")],
  ["area", "area (kpx²)", (v) => Math.round(v / 1000).toLocaleString("en-US")],
];
const names = Object.keys(engines);
const wins = Object.fromEntries(names.map((e) => [e, Object.fromEntries(metrics.map(([k]) => [k, 0]))]));
const totals = Object.fromEntries(names.map((e) => [e, Object.fromEntries(metrics.map(([k]) => [k, 0]))]));
for (const row of rows) {
  for (const [key] of metrics) {
    const values = names.map((e) => row.results[e]?.[key]).filter((v) => typeof v === "number");
    if (values.length !== names.length) continue;
    const best = Math.min(...values);
    for (const e of names) {
      totals[e][key] += row.results[e][key];
      if (row.results[e][key] === best) wins[e][key] += 1;
    }
  }
}

let md = `# Benchmarks

The corpus is every plain fixture of the test suite: process models discovered from public event
logs and synthetic graphs, from a few nodes to two thousand. graphlay,
[dagre](https://github.com/dagrejs/dagre) and [ELK](https://eclipse.dev/elk/) (layered, orthogonal
routing) each lay out every case with the same node sizes and the same node and layer gaps.
graphlay uses the options the fixture names, which for a directly-follows graph is the process-map
preset. graphlay's own metrics measure all three results.

Lower is better everywhere. Time is the median of several runs in Node, graphlay as WebAssembly,
the others as JavaScript. Area is the bounding box of nodes and edges. "graphlay fast" is the same
engine with \`effort: "fast"\`, the setting for an interactive relayout.

dagre's edge points are control points for a smooth curve. Counted as a polyline they bend less
and cut through more nodes than the drawn curve would. On a directly-follows graph, graphlay uses
edge weights to keep the main path straight, which costs it bends and saves crossings. dagre and
ELK do not read weights that way.

Versions: graphlay ${versions.graphlay}, @dagrejs/dagre ${versions.dagre}, elkjs ${versions.elk}.
Regenerate with \`pnpm --filter bench bench\`.

## Over the whole corpus

How many cases each engine wins, with a tie counting for every engine that shares it, and the
totals.

| engine | ${metrics.map(([, label]) => `${label} wins`).join(" | ")} |
|---|${metrics.map(() => "---:").join("|")}|
`;
for (const e of names) {
  md += `| ${e} | ${metrics.map(([k]) => `${wins[e][k]} of ${rows.length}`).join(" | ")} |\n`;
}
md += `
| engine | ${metrics.map(([, label]) => `total ${label}`).join(" | ")} |
|---|${metrics.map(() => "---:").join("|")}|
`;
for (const e of names) {
  md += `| ${e} | ${metrics.map(([k, , fmt]) => fmt(totals[e][k])).join(" | ")} |\n`;
}
md += `
## Case by case

| case | nodes | edges | engine | ${metrics.map(([, label]) => label).join(" | ")} |
|---|---:|---:|---|${metrics.map(() => "---:").join("|")}|
`;
for (const row of rows) {
  for (const e of names) {
    const r = row.results[e];
    const cells = r.error ? metrics.map(() => "failed") : metrics.map(([k, , fmt]) => fmt(r[k]));
    md += `| ${row.name} | ${row.nodes} | ${row.edges} | ${e} | ${cells.join(" | ")} |\n`;
  }
}
writeFileSync("../docs/benchmarks.md", md);
writeFileSync("results.json", JSON.stringify({ versions, rows }, null, 2) + "\n");
console.log(`wrote docs/benchmarks.md with ${rows.length} cases`);
