// The package as a user imports it, under Node.
import assert from "node:assert/strict";
import { test } from "node:test";

import { GraphlayError, layout, load } from "../dist/index.js";

const box = { size: { width: 80, height: 40 } };
const graph = { nodes: [box, box, box], edges: [{ source: 0, target: 1 }, { source: 0, target: 2 }] };

test("a graph is laid out, left to right by default", async () => {
  const out = await layout({ graph });
  assert.equal(out.nodes.length, 3);
  assert.equal(out.edges.length, 2);
  assert.ok(out.nodes[0].center.x < out.nodes[1].center.x);
});

test("the loaded engine answers synchronously", async () => {
  const engine = await load();
  assert.match(engine.version, /^\d+\.\d+\.\d+/);
  const out = engine.layout({ graph, preset: "process_map", options: { routing: { style: "diagonal" } } });
  assert.ok(out.nodes[0].center.y < out.nodes[1].center.y, "process maps run top to bottom");
  assert.equal(engine.fingerprint(out).length, 16);
  assert.equal(engine.metrics({ graph, preset: "process_map" }, out).crossings, 0);
  assert.deepEqual(engine.presets(), ["layered", "process_map", "hub_and_spoke", "hub_and_spoke_flow"]);
  assert.equal(engine.options("hub_and_spoke").direction, engine.options("hub_and_spoke").direction);
  const tree = engine.layoutTree({ graph });
  assert.ok(tree.nodes[0].center.y < tree.nodes[1].center.y);
  assert.ok(engine.renderScene({ nodes: [], edges: [] }).includes("<svg"));
  const options = { labels: ["one", "two", "three"] };
  const sketch = engine.sketch({ graph }, out, options);
  assert.ok(sketch.includes("<svg") && sketch.includes("three"));
  assert.equal(engine.renderScene(engine.sketchScene({ graph }, out, options)), sketch);
});

test("a reroute keeps the positions it was given", async () => {
  const engine = await load();
  const first = engine.layout({ graph });
  const moved = structuredClone(graph);
  moved.nodes = moved.nodes.map((n, i) => ({ ...n, position: first.nodes[i].center }));
  moved.nodes[2].position = { x: moved.nodes[2].position.x + 100, y: moved.nodes[2].position.y };
  const out = engine.reroute({ graph: moved });
  assert.equal(out.nodes[2].center.x, first.nodes[2].center.x + 100);
});

test("a bad option is a GraphlayError naming the field", async () => {
  const engine = await load();
  assert.throws(
    () => engine.layout({ graph, options: { routing: { style: "curvy" } } }),
    (e) => e instanceof GraphlayError && e.message.startsWith("options.routing.style"),
  );
  await assert.rejects(layout({ graph: { nodes: [box], edges: [{ source: 0, target: 5 }] } }), GraphlayError);
});
