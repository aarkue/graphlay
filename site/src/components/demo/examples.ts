// The graphs the demo starts from. Each is a complete request, so the JSON drawer shows exactly
// what the engine receives.
import type { Edge, Graph, LayoutRequest, Node } from "graphlay";

export interface Example {
  name: string;
  request: LayoutRequest;
  /** Lane names by group id, for drawings with swimlanes. */
  laneNames?: string[];
}

const box = (key: string, width = 120, height = 44, rest: Partial<Node> = {}): Node => ({
  key,
  size: { width, height },
  ...rest,
});
const circle = (key: string, d = 36): Node => ({ key, shape: "ellipse", size: { width: d, height: d } });
const edge = (source: number, target: number, count = 1): Edge => ({
  source,
  target,
  weight: 1 + Math.log(count),
  thickness: Math.min(1.5 + Math.log(count), 6),
});

function processMap(): LayoutRequest {
  const names = [
    "Receive order",
    "Check credit",
    "Reserve stock",
    "Pick items",
    "Pack",
    "Ship",
    "Send invoice",
    "Receive payment",
    "Close order",
    "Reject order",
  ];
  const nodes = names.map((n) => box(n, Math.max(100, 8 * n.length + 24), 44));
  const edges = [
    edge(0, 1, 420),
    edge(1, 2, 380),
    edge(1, 9, 40),
    edge(2, 3, 380),
    edge(3, 4, 375),
    edge(4, 5, 375),
    edge(5, 6, 370),
    edge(6, 7, 340),
    edge(7, 8, 340),
    edge(2, 6, 25),
    edge(7, 6, 30),
    edge(9, 8, 40),
    edge(0, 2, 12),
  ];
  return { graph: { nodes, edges }, preset: "process_map", options: { routing: { style: "diagonal" } } };
}

function petriNet(): LayoutRequest {
  const nodes: Node[] = [
    circle("p0"),
    box("register", 110, 40),
    circle("p1"),
    box("examine", 110, 40),
    box("check ticket", 110, 40),
    circle("p2"),
    circle("p3"),
    box("decide", 110, 40),
    circle("p4"),
    box("reinitiate", 110, 40),
    box("pay", 110, 40),
    box("reject", 110, 40),
    circle("p5"),
  ];
  const edges: Edge[] = [
    [0, 1], [1, 2], [2, 3], [2, 4], [3, 5], [4, 6], [5, 7], [6, 7], [7, 8],
    [8, 9], [9, 2], [8, 10], [8, 11], [10, 12], [11, 12],
  ].map(([s, t]) => ({ source: s, target: t, weight: 1, thickness: 1.6 }));
  return { graph: { nodes, edges }, preset: "layered", options: {} };
}

function hubAndSpoke(): LayoutRequest {
  const types = ["Order", "Item", "Package", "Customer", "Invoice", "Payment", "Route", "Truck", "Employee", "Product", "Supplier"];
  const nodes = types.map((t) => box(t, 100, 40));
  const edges: Edge[] = [];
  for (let i = 1; i < types.length; i++) {
    edges.push({ source: 0, target: i, weight: 1 + (i % 3), thickness: 1.5 });
    if (i % 2 === 0) edges.push({ source: i, target: 0, weight: 1, thickness: 1.5 });
    if (i + 1 < types.length && i % 3 === 1) edges.push({ source: i, target: i + 1, weight: 1, thickness: 1.5 });
  }
  return { graph: { nodes, edges }, preset: "hub_and_spoke", options: {} };
}

function nested(): LayoutRequest {
  const nodes: Node[] = [
    box("Start", 80, 40),
    box("Order handling", 10, 10),
    box("Validate", 100, 40, { parent: 1 }),
    box("Price", 100, 40, { parent: 1 }),
    box("Approve", 100, 40, { parent: 1 }),
    box("Fulfilment", 10, 10),
    box("Pick", 100, 40, { parent: 5 }),
    box("Ship", 100, 40, { parent: 5 }),
    box("End", 80, 40),
  ];
  const edges: Edge[] = [
    [0, 2], [2, 3], [3, 4], [4, 6], [6, 7], [7, 8], [4, 8],
  ].map(([s, t]) => ({ source: s, target: t, weight: 1, thickness: 1.6 }));
  return {
    graph: { nodes, edges },
    preset: "layered",
    options: { spacing: { compound_header: 22 } },
  };
}

function wrappedChain(): LayoutRequest {
  const steps = 16;
  const nodes = Array.from({ length: steps }, (_, i) => box(`Step ${i + 1}`, 110, 40));
  const edges = Array.from({ length: steps - 1 }, (_, i) => edge(i, i + 1, 100));
  edges.push(edge(3, 9, 8), edge(12, 5, 5));
  return {
    graph: { nodes, edges },
    preset: "process_map",
    options: { direction: "leftright", wrap: { viewport_aspect: 1.78 } },
  };
}

function swimlanes(): Example {
  const lane = (key: string, group: number) => box(key, 110, 40, { group });
  const nodes = [
    lane("Request", 0), lane("Review", 1), lane("Clarify", 0), lane("Approve", 2),
    lane("Reject", 1), lane("Execute", 2), lane("Report", 0), lane("Archive", 2),
  ];
  const edges: Edge[] = [
    [0, 1], [1, 2], [2, 1], [1, 3], [1, 4], [3, 5], [5, 6], [6, 7], [4, 6],
  ].map(([s, t]) => ({ source: s, target: t, weight: 1, thickness: 1.6 }));
  return {
    name: "Swimlanes",
    laneNames: ["Customer", "Clerk", "Manager"],
    request: {
      graph: { nodes, edges },
      preset: "process_map",
      options: { direction: "leftright", order: { group_lanes: "strict" } },
    },
  };
}

function automaton(): LayoutRequest {
  const nodes = Array.from({ length: 5 }, (_, i) => circle(`q${i}`, 44));
  const edges: Edge[] = [
    [0, 1], [0, 0], [1, 1], [1, 2], [2, 3], [2, 0], [3, 4], [3, 3], [4, 4],
  ].map(([s, t]) => ({ source: s, target: t, weight: 1, thickness: 1.6 }));
  return { graph: { nodes, edges }, preset: "layered", options: { self_loops: { side: "cross_negative" } } };
}

function serviceMap(): LayoutRequest {
  const names = ["Web app", "Mobile API", "Auth", "Orders", "Catalog", "Payments", "Notify", "Users DB", "Orders DB", "Cache", "Queue"];
  const nodes = names.map((n, i) => (i >= 7 ? { key: n, shape: "ellipse" as const, size: { width: 96, height: 40 } } : box(n, 104, 40)));
  const edges = [
    [0, 2, 120], [0, 3, 300], [0, 4, 900], [1, 2, 200], [1, 3, 150], [3, 5, 140], [3, 4, 300], [3, 6, 100],
    [5, 6, 90], [2, 7, 320], [3, 8, 450], [4, 9, 1200], [4, 8, 60], [6, 10, 190], [5, 8, 140],
  ].map(([s, t, rate]) => edge(s, t, rate / 10));
  return { graph: { nodes, edges }, preset: "layered", options: { direction: "leftright", spacing: { layer_gap: 100 } } };
}

/** A random directed graph with a flow and a few back edges, the same for the same seed. */
export function randomGraph(nodeCount: number, seed: number): LayoutRequest {
  let state = (seed >>> 0) || 1;
  const next = () => {
    // xorshift32
    state ^= state << 13;
    state >>>= 0;
    state ^= state >>> 17;
    state ^= state << 5;
    state >>>= 0;
    return state / 4294967296;
  };
  const nodes = Array.from({ length: nodeCount }, (_, i) => box(`N${i + 1}`, 80 + Math.floor(next() * 50), 40));
  const edges: Edge[] = [];
  const seen = new Set<string>();
  const add = (s: number, t: number, count: number) => {
    if (s === t || seen.has(`${s}-${t}`)) return;
    seen.add(`${s}-${t}`);
    edges.push(edge(s, t, count));
  };
  for (let i = 1; i < nodeCount; i++) {
    add(Math.floor(next() * i), i, 1 + Math.floor(next() * 60));
  }
  const extra = Math.floor(nodeCount * 0.6);
  for (let k = 0; k < extra; k++) {
    const s = Math.floor(next() * nodeCount);
    const t = Math.floor(next() * nodeCount);
    add(Math.min(s, t), Math.max(s, t), 1 + Math.floor(next() * 20));
  }
  if (nodeCount > 4) add(nodeCount - 1, Math.floor(next() * 3), 3);
  return { graph: { nodes, edges }, preset: "process_map", options: {} };
}

export const examples: Example[] = [
  { name: "Process map", request: processMap() },
  { name: "Petri net", request: petriNet() },
  { name: "Hub and spoke", request: hubAndSpoke() },
  { name: "Nested subprocesses", request: nested() },
  { name: "Wrapped chain", request: wrappedChain() },
  swimlanes(),
  { name: "Finite automaton", request: automaton() },
  { name: "Service map", request: serviceMap() },
  { name: "Random graph", request: randomGraph(24, 7) },
];
