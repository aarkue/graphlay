// The interactive demo: pick or paste a graph, change the options, drag nodes and watch the
// edges follow, and read the quality metrics of the result.
import type { Engine, Layout, LayoutMetrics, LayoutRequest, Point, Preset } from "graphlay";
import { GraphlayError, load } from "graphlay";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { Canvas } from "./Canvas";
import styles from "./demo.module.css";
import { examples, randomGraph } from "./examples";

interface Drawn {
  /** The request the drawing answers, which the canvas and a drag must use. */
  request: LayoutRequest;
  layout: Layout;
  metrics: LayoutMetrics;
  ms: number;
}

/** The request with every node at `positions`, for a reroute or a seeded relayout. */
function withPositions(request: LayoutRequest, positions: Point[]): LayoutRequest {
  return {
    ...request,
    graph: {
      ...request.graph,
      nodes: request.graph.nodes.map((n, i) => ({ ...n, position: positions[i] })),
    },
  };
}

function withoutPositions(request: LayoutRequest): LayoutRequest {
  return {
    ...request,
    graph: {
      ...request.graph,
      nodes: request.graph.nodes.map(({ position: _position, ...n }) => n),
    },
  };
}

function patch(request: LayoutRequest, options: NonNullable<LayoutRequest["options"]>): LayoutRequest {
  const merge = (base: Record<string, unknown>, more: Record<string, unknown>): Record<string, unknown> => {
    const out = { ...base };
    for (const [k, v] of Object.entries(more)) {
      const prev = out[k];
      out[k] =
        v && typeof v === "object" && !Array.isArray(v) && prev && typeof prev === "object"
          ? merge(prev as Record<string, unknown>, v as Record<string, unknown>)
          : v;
    }
    return out;
  };
  return { ...request, options: merge((request.options ?? {}) as Record<string, unknown>, options as Record<string, unknown>) };
}

const metricRows: [keyof LayoutMetrics, string][] = [
  ["crossings", "Crossings"],
  ["bends", "Bends"],
  ["node_edge_hits", "Edges through nodes"],
  ["overlaps", "Overlapping segments"],
  ["short_steps", "Short steps"],
  ["staircase_steps", "Staircase steps"],
  ["total_edge_length", "Edge length"],
];

export default function Demo(): ReactNode {
  const [engine, setEngine] = useState<Engine | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [exampleIndex, setExampleIndex] = useState(0);
  const [request, setRequest] = useState<LayoutRequest>(examples[0].request);
  const [drawn, setDrawn] = useState<Drawn | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [fitKey, setFitKey] = useState(0);
  const [showJson, setShowJson] = useState(false);
  const [jsonText, setJsonText] = useState("");
  const [randomSize, setRandomSize] = useState(24);
  // Where the nodes are drawn. A drag moves these at once. The engine's answer replaces them
  // after a layout and when the drag ends.
  const [positions, setPositions] = useState<Point[]>([]);
  const positionsRef = useRef<Point[]>([]);
  const pending = useRef<number | null>(null);
  const place = useCallback((next: Point[]) => {
    positionsRef.current = next;
    setPositions(next);
  }, []);
  const laneNames = examples[exampleIndex]?.laneNames;

  useEffect(() => {
    load().then(setEngine, (e: unknown) => setLoadError(e instanceof Error ? e.message : String(e)));
  }, []);

  /**
   * Runs the engine and keeps the result, or the error it reported. With `settle`, the engine's
   * node positions replace the drawn ones. A re-route during a drag leaves them alone.
   */
  const draw = useCallback(
    (run: (engine: Engine) => Layout, req: LayoutRequest, settle: boolean, refit: boolean) => {
      if (!engine) return;
      try {
        const t0 = performance.now();
        const layout = run(engine);
        const ms = performance.now() - t0;
        const metrics = engine.metrics(req, layout);
        setDrawn({ request: req, layout, metrics, ms });
        if (settle) place(layout.nodes.map((n) => n.center));
        setError(null);
        if (refit) setFitKey((k) => k + 1);
      } catch (e) {
        setError(e instanceof GraphlayError ? e.message : String(e));
      }
    },
    [engine, place],
  );

  // A fresh layout whenever the request changes.
  useEffect(() => {
    if (!engine) return;
    const fresh = withoutPositions(request);
    draw((eng) => eng.layout(fresh), fresh, true, true);
    setJsonText(JSON.stringify(fresh, null, 2));
  }, [engine, request, draw]);

  const onDrag = (node: number, to: Point) => {
    if (!drawn) return;
    const next = positionsRef.current.slice();
    next[node] = to;
    place(next);
    if (pending.current !== null) return;
    const base = drawn.request;
    pending.current = requestAnimationFrame(() => {
      pending.current = null;
      const seeded = withPositions(base, positionsRef.current);
      draw((eng) => eng.reroute(seeded), seeded, false, false);
    });
  };

  /** On release, re-routes once more and takes the engine's node positions. */
  const onDragEnd = () => {
    if (!drawn) return;
    if (pending.current !== null) {
      cancelAnimationFrame(pending.current);
      pending.current = null;
    }
    const seeded = withPositions(drawn.request, positionsRef.current);
    draw((eng) => eng.reroute(seeded), seeded, true, false);
  };

  const tidy = () => {
    if (!drawn) return;
    const seeded = patch(withPositions(drawn.request, positionsRef.current), { seed_mode: "structure" });
    draw((eng) => eng.layout(seeded), seeded, true, false);
  };

  const applyJson = () => {
    try {
      const parsed = JSON.parse(jsonText) as LayoutRequest;
      if (!parsed || typeof parsed !== "object" || !parsed.graph) throw new Error("the request needs a graph");
      setRequest(parsed);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const download = () => {
    if (!engine || !drawn) return;
    const svg = engine.sketch(withPositions(drawn.request, positionsRef.current), drawn.layout, {
      lane_names: laneNames ?? [],
    });
    const url = URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = "graphlay.svg";
    a.click();
    URL.revokeObjectURL(url);
  };

  const options = request.options ?? {};
  const presets = useMemo(() => engine?.presets() ?? ["layered", "process_map", "hub_and_spoke", "hub_and_spoke_flow"], [engine]);
  const presetDefaults = useMemo(
    () => (engine ? engine.options(request.preset ?? "layered") : null),
    [engine, request.preset],
  );
  const direction = options.direction ?? presetDefaults?.direction ?? "leftright";
  const style = options.routing?.style ?? presetDefaults?.routing?.style ?? "orthogonal";
  const merge = options.routing?.merge_edges ?? presetDefaults?.routing?.merge_edges ?? "none";
  const layerGap = options.spacing?.layer_gap ?? presetDefaults?.spacing?.layer_gap ?? 54;
  const nodeGap = options.spacing?.node_gap ?? presetDefaults?.spacing?.node_gap ?? 30;
  const effortValue = options.effort ?? presetDefaults?.effort ?? "balanced";
  // Effort is a named level or a custom budget. The control shows the levels.
  const effort = typeof effortValue === "string" ? effortValue : "custom";
  const seed = options.seed ?? presetDefaults?.seed ?? 0;

  if (loadError) {
    return <p className={styles.error}>The engine failed to load: {loadError}</p>;
  }

  return (
    <div className={styles.demo}>
      <aside className={styles.panel}>
        <label className={styles.field}>
          <span>Example</span>
          <select
            value={exampleIndex}
            onChange={(e) => {
              const i = Number(e.target.value);
              setExampleIndex(i);
              setRequest(examples[i].request);
            }}
          >
            {examples.map((ex, i) => (
              <option key={ex.name} value={i}>
                {ex.name}
              </option>
            ))}
          </select>
        </label>
        <div className={styles.row}>
          <label className={styles.field}>
            <span>Random nodes</span>
            <input type="number" min={3} max={300} value={randomSize} onChange={(e) => setRandomSize(Number(e.target.value))} />
          </label>
          <button
            type="button"
            onClick={() => {
              setExampleIndex(examples.length - 1);
              setRequest(randomGraph(randomSize, Math.floor(Math.random() * 1e9)));
            }}
          >
            Generate
          </button>
        </div>
        <hr />
        <label className={styles.field}>
          <span>Preset</span>
          <select value={request.preset ?? "layered"} onChange={(e) => setRequest({ ...request, preset: e.target.value as Preset })}>
            {presets.map((p) => (
              <option key={p} value={p}>
                {p}
              </option>
            ))}
          </select>
        </label>
        <label className={styles.field}>
          <span>Direction</span>
          <select value={direction} onChange={(e) => setRequest(patch(request, { direction: e.target.value as "leftright" | "topbottom" }))}>
            <option value="leftright">left to right</option>
            <option value="topbottom">top to bottom</option>
          </select>
        </label>
        <label className={styles.field}>
          <span>Routing</span>
          <select value={style} onChange={(e) => setRequest(patch(request, { routing: { style: e.target.value as "orthogonal" | "diagonal" } }))}>
            <option value="orthogonal">orthogonal</option>
            <option value="diagonal">diagonal</option>
          </select>
        </label>
        <label className={styles.field}>
          <span>Merge edges</span>
          <select value={merge} onChange={(e) => setRequest(patch(request, { routing: { merge_edges: e.target.value as "none" | "forks" | "joins" | "both" } }))}>
            <option value="none">none</option>
            <option value="forks">forks</option>
            <option value="joins">joins</option>
            <option value="both">forks and joins</option>
          </select>
        </label>
        <label className={styles.field}>
          <span>Layer gap: {layerGap}px</span>
          <input type="range" min={20} max={200} value={layerGap} onChange={(e) => setRequest(patch(request, { spacing: { layer_gap: Number(e.target.value) } }))} />
        </label>
        <label className={styles.field}>
          <span>Node gap: {nodeGap}px</span>
          <input type="range" min={8} max={120} value={nodeGap} onChange={(e) => setRequest(patch(request, { spacing: { node_gap: Number(e.target.value) } }))} />
        </label>
        <label className={styles.field}>
          <span>Effort</span>
          <select value={effort} onChange={(e) => setRequest(patch(request, { effort: e.target.value as "fast" | "balanced" | "thorough" }))}>
            <option value="fast">fast</option>
            <option value="balanced">balanced</option>
            <option value="thorough">thorough</option>
            {effort === "custom" && <option value="custom">custom budget</option>}
          </select>
        </label>
        <label className={styles.field}>
          <span>Seed</span>
          <input type="number" min={0} value={seed} onChange={(e) => setRequest(patch(request, { seed: Math.max(0, Math.floor(Number(e.target.value) || 0)) }))} />
        </label>
        <hr />
        <div className={styles.buttons}>
          <button type="button" onClick={tidy} disabled={!drawn} title="Lay out again, keeping the layers and the order the nodes have now">
            Tidy
          </button>
          <button type="button" onClick={() => setRequest({ ...request })} disabled={!engine} title="Lay out again from scratch">
            Relayout
          </button>
          <button type="button" onClick={() => setFitKey((k) => k + 1)} disabled={!drawn}>
            Fit
          </button>
          <button type="button" onClick={download} disabled={!drawn}>
            Download SVG
          </button>
          <button type="button" onClick={() => setShowJson((s) => !s)}>
            {showJson ? "Hide JSON" : "Edit JSON"}
          </button>
        </div>
        {drawn && (
          <table className={styles.metrics}>
            <tbody>
              {metricRows.map(([key, label]) => {
                const value = drawn.metrics[key];
                if (value === undefined || value === null) return null;
                return (
                  <tr key={key}>
                    <td>{label}</td>
                    <td>{typeof value === "number" ? (Number.isInteger(value) ? value : Math.round(value)) : String(value)}</td>
                  </tr>
                );
              })}
              <tr>
                <td>Size</td>
                <td>
                  {Math.round(drawn.metrics.width)} x {Math.round(drawn.metrics.height)}
                </td>
              </tr>
              <tr>
                <td>Layers</td>
                <td>{drawn.layout.layer_count}</td>
              </tr>
              <tr>
                <td>Time</td>
                <td>{drawn.ms < 1 ? "<1" : Math.round(drawn.ms)} ms</td>
              </tr>
            </tbody>
          </table>
        )}
        <p className={styles.hint}>
          Drag a node and the edges are re-routed around the others. Scroll to zoom, drag the background to pan.
          {engine && <> Engine {engine.version}.</>}
        </p>
      </aside>
      <section className={styles.stage}>
        {error && <p className={styles.error}>{error}</p>}
        {!engine && !loadError && <p className={styles.hint}>Loading the engine...</p>}
        {drawn && positions.length === drawn.layout.nodes.length && (
          <Canvas
            graph={drawn.request.graph}
            layout={drawn.layout}
            positions={positions}
            laneNames={laneNames}
            fitKey={fitKey}
            onDrag={onDrag}
            onDragEnd={onDragEnd}
          />
        )}
        {showJson && (
          <div className={styles.json}>
            <textarea value={jsonText} onChange={(e) => setJsonText(e.target.value)} spellCheck={false} />
            <div className={styles.buttons}>
              <button type="button" onClick={applyJson}>
                Apply
              </button>
              <button type="button" onClick={() => navigator.clipboard?.writeText(jsonText)}>
                Copy
              </button>
            </div>
          </div>
        )}
      </section>
    </div>
  );
}
