// Draws a layout as SVG, with pan, zoom and node dragging. During a drag, nodes are drawn at
// `positions`, ahead of the layout, and the edge ends at a moved node shift with it.
import type { Graph, Layout, Point } from "graphlay";
import { useCallback, useEffect, useRef, useState, type PointerEvent, type ReactNode, type WheelEvent } from "react";

import styles from "./demo.module.css";

const EDGE_COLOURS = ["#2563eb", "#dc2626", "#059669", "#d97706", "#7c3aed", "#db2777"];
const DEFAULT_EDGE = "#3b4658";

interface ViewBox {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface CanvasProps {
  graph: Graph;
  layout: Layout;
  /** Every node's centre as drawn. Between drags it matches the layout. */
  positions: Point[];
  laneNames?: string[];
  /** Bumped when the view should fit the drawing again. */
  fitKey: number;
  onDrag: (node: number, to: Point) => void;
  onDragEnd: () => void;
}

/** A path with every corner rounded by `r`, like the engine's renderer draws it. */
function roundedPath(points: Point[], r: number): string {
  if (points.length < 2) return "";
  let d = `M${points[0].x} ${points[0].y}`;
  for (let i = 1; i < points.length - 1; i++) {
    const p0 = points[i - 1];
    const p1 = points[i];
    const p2 = points[i + 1];
    const a = Math.hypot(p1.x - p0.x, p1.y - p0.y);
    const b = Math.hypot(p2.x - p1.x, p2.y - p1.y);
    const ra = Math.min(r, a / 2);
    const rb = Math.min(r, b / 2);
    if (a < 1e-6 || b < 1e-6) continue;
    const inX = p1.x - ((p1.x - p0.x) / a) * ra;
    const inY = p1.y - ((p1.y - p0.y) / a) * ra;
    const outX = p1.x + ((p2.x - p1.x) / b) * rb;
    const outY = p1.y + ((p2.y - p1.y) / b) * rb;
    d += ` L${inX} ${inY} Q${p1.x} ${p1.y} ${outX} ${outY}`;
  }
  const last = points[points.length - 1];
  d += ` L${last.x} ${last.y}`;
  return d;
}

function edgeWidth(thickness: number | undefined): number {
  return Math.min(Math.max(thickness ?? 1.6, 1.5), 4.5);
}

/** The arrowhead grows with the line, so a thick edge does not end in a pin. */
function arrowSize(width: number): number {
  return Math.round(9 + 2 * width);
}

function fit(layout: Layout, pad: number): ViewBox {
  const b = layout.bounds;
  return {
    x: b.min.x - pad,
    y: b.min.y - pad,
    w: Math.max(b.max.x - b.min.x + 2 * pad, 10),
    h: Math.max(b.max.y - b.min.y + 2 * pad, 10),
  };
}

export function Canvas({ graph, layout, positions, laneNames, fitKey, onDrag, onDragEnd }: CanvasProps): ReactNode {
  const svgRef = useRef<SVGSVGElement>(null);
  const [view, setView] = useState<ViewBox>(() => fit(layout, 40));
  const [dragging, setDragging] = useState<number | null>(null);
  const gesture = useRef<
    | { kind: "pan"; x: number; y: number; view: ViewBox }
    | { kind: "node"; node: number; dx: number; dy: number }
    | null
  >(null);

  useEffect(() => {
    setView(fit(layout, 40));
    // Refit only when `fitKey` changes, not on every reroute.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fitKey]);

  /** The point under the pointer, in drawing coordinates. The view box scales uniformly, so one
   * scale serves both axes. */
  const toDrawing = useCallback(
    (e: { clientX: number; clientY: number }): Point => {
      const svg = svgRef.current;
      if (!svg) return { x: 0, y: 0 };
      const rect = svg.getBoundingClientRect();
      const scale = Math.max(view.w / rect.width, view.h / rect.height);
      const slackX = (rect.width * scale - view.w) / 2;
      const slackY = (rect.height * scale - view.h) / 2;
      return {
        x: view.x - slackX + (e.clientX - rect.left) * scale,
        y: view.y - slackY + (e.clientY - rect.top) * scale,
      };
    },
    [view],
  );

  const onWheel = (e: WheelEvent<SVGSVGElement>) => {
    e.preventDefault();
    const at = toDrawing(e);
    const factor = e.deltaY > 0 ? 1.15 : 1 / 1.15;
    setView((v) => ({
      x: at.x - (at.x - v.x) * factor,
      y: at.y - (at.y - v.y) * factor,
      w: v.w * factor,
      h: v.h * factor,
    }));
  };

  const onPointerDown = (e: PointerEvent<SVGSVGElement>) => {
    const target = e.target as Element;
    const nodeAttr = target.closest("[data-node]")?.getAttribute("data-node");
    const at = toDrawing(e);
    if (nodeAttr !== null && nodeAttr !== undefined) {
      const node = Number(nodeAttr);
      const c = positions[node];
      gesture.current = { kind: "node", node, dx: c.x - at.x, dy: c.y - at.y };
      setDragging(node);
    } else {
      gesture.current = { kind: "pan", x: e.clientX, y: e.clientY, view };
    }
    svgRef.current?.setPointerCapture(e.pointerId);
  };

  const onPointerMove = (e: PointerEvent<SVGSVGElement>) => {
    const g = gesture.current;
    if (!g) return;
    if (g.kind === "pan") {
      const svg = svgRef.current;
      if (!svg) return;
      const rect = svg.getBoundingClientRect();
      const scale = Math.max(g.view.w / rect.width, g.view.h / rect.height);
      setView({ ...g.view, x: g.view.x - (e.clientX - g.x) * scale, y: g.view.y - (e.clientY - g.y) * scale });
    } else {
      const at = toDrawing(e);
      onDrag(g.node, { x: at.x + g.dx, y: at.y + g.dy });
    }
  };

  const onPointerUp = (e: PointerEvent<SVGSVGElement>) => {
    const g = gesture.current;
    gesture.current = null;
    svgRef.current?.releasePointerCapture(e.pointerId);
    if (g?.kind === "node") {
      setDragging(null);
      onDragEnd();
    }
  };

  /** How far a node is drawn from where the layout last put it. */
  const shift = (node: number): Point => ({
    x: positions[node].x - layout.nodes[node].center.x,
    y: positions[node].y - layout.nodes[node].center.y,
  });

  const isCompound = (i: number) => graph.nodes.some((n) => n.parent === i);
  const depth = (i: number) => {
    let d = 0;
    let at: number | null | undefined = graph.nodes[i].parent;
    while (at !== null && at !== undefined) {
      d += 1;
      at = graph.nodes[at].parent;
    }
    return d;
  };
  const order = layout.nodes.map((_, i) => i).sort((a, b) => depth(a) - depth(b) || a - b);
  const colourOf = (edge: number) => {
    const g = graph.edges[edge].group;
    return g !== null && g !== undefined ? EDGE_COLOURS[g % EDGE_COLOURS.length] : DEFAULT_EDGE;
  };
  const markers = new Map<string, { colour: string; size: number }>();
  graph.edges.forEach((e, i) => {
    const colour = colourOf(i);
    const size = arrowSize(edgeWidth(e.thickness));
    markers.set(`${colour.slice(1)}-${size}`, { colour, size });
  });

  return (
    <svg
      ref={svgRef}
      className={styles.canvas}
      viewBox={`${view.x} ${view.y} ${view.w} ${view.h}`}
      onWheel={onWheel}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
    >
      <defs>
        {[...markers.entries()].map(([key, { colour, size }]) => (
          <marker
            key={key}
            id={`arrow-${key}`}
            markerWidth={size}
            markerHeight={size}
            refX={size - 1}
            refY={size / 2}
            orient="auto"
            markerUnits="userSpaceOnUse"
          >
            <path d={`M0 0.5 L${size - 0.5} ${size / 2} L0 ${size - 0.5} L${size * 0.22} ${size / 2} Z`} fill={colour} />
          </marker>
        ))}
      </defs>
      {(layout.lanes ?? []).map((lane, i) => (
        <g key={`lane-${i}`} className={styles.lane}>
          <rect
            x={lane.rect.min.x}
            y={lane.rect.min.y}
            width={lane.rect.max.x - lane.rect.min.x}
            height={lane.rect.max.y - lane.rect.min.y}
            fill={i % 2 === 0 ? "#f3f5f9" : "#e9edf4"}
          />
          <text x={lane.rect.min.x + 8} y={lane.rect.min.y + 16}>
            {lane.group !== null && lane.group !== undefined ? laneNames?.[lane.group] ?? `Lane ${lane.group + 1}` : "Other"}
          </text>
        </g>
      ))}
      {layout.edges.map((route, i) => {
        if (route.points.length < 2) return null;
        const e = graph.edges[i];
        const colour = colourOf(i);
        const width = edgeWidth(e.thickness);
        const from = shift(e.source);
        const to = shift(e.target);
        let points = route.points;
        if (from.x !== 0 || from.y !== 0 || to.x !== 0 || to.y !== 0) {
          const last = route.points.length - 1;
          points = route.points.map((p, k) => {
            if (e.source === e.target) return { x: p.x + from.x, y: p.y + from.y };
            if (k === 0) return { x: p.x + from.x, y: p.y + from.y };
            if (k === last) return { x: p.x + to.x, y: p.y + to.y };
            return p;
          });
        }
        return (
          <path
            key={`edge-${i}`}
            d={roundedPath(points, 6)}
            fill="none"
            stroke={colour}
            strokeWidth={width}
            strokeLinecap="round"
            strokeLinejoin="round"
            markerEnd={`url(#arrow-${colour.slice(1)}-${arrowSize(width)})`}
          />
        );
      })}
      {(layout.junctions ?? []).map((p, i) => (
        <circle key={`junction-${i}`} cx={p.x} cy={p.y} r={3} fill={DEFAULT_EDGE} />
      ))}
      {order.map((i) => {
        const n = graph.nodes[i];
        const at = positions[i];
        const { width: w, height: h } = layout.nodes[i].size;
        const compound = isCompound(i);
        const label = n.key ?? "";
        // A transform moves the whole group, and it animates when the node settles after a drag.
        const classes = [compound ? styles.compound : styles.node, dragging === i ? styles.dragging : ""].join(" ");
        return (
          <g key={`node-${i}`} data-node={compound ? undefined : i} className={classes} transform={`translate(${at.x} ${at.y})`}>
            {n.shape === "ellipse" ? (
              <ellipse cx={0} cy={0} rx={w / 2} ry={h / 2} />
            ) : (
              <rect x={-w / 2} y={-h / 2} width={w} height={h} rx={6} />
            )}
            <text x={0} y={compound ? -h / 2 + 16 : 0} textAnchor="middle" dominantBaseline={compound ? "auto" : "central"}>
              {label}
            </text>
          </g>
        );
      })}
    </svg>
  );
}
