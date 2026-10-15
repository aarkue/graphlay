# graphlay

Deterministic graph layout for the browser and Node: a layered (Sugiyama-style) engine with
orthogonal or diagonal edge routing, a tidy-tree layout, quality metrics and an SVG renderer. The
engine is [graphlay](https://github.com/aarkue/graphlay), Rust compiled to WebAssembly. The
wasm is embedded in the package, so a bundler needs no extra configuration.

```sh
npm install graphlay
```

```ts
import { layout } from "graphlay";

const box = { size: { width: 120, height: 50 } };
const drawing = await layout({
  graph: { nodes: [box, box, box], edges: [{ source: 0, target: 1 }, { source: 1, target: 2 }] },
  preset: "process_map",
  options: { routing: { style: "diagonal" } },
});
drawing.nodes[0].center; // { x, y }
drawing.edges[0].points; // the polyline, border to border
```

`load()` returns the engine once for synchronous calls, which suits a drag handler:

```ts
import { load } from "graphlay";

const engine = await load();
const rerouted = engine.reroute({ graph: graphWithPositions });
```

`sketch(request, drawing)` draws a layout as plain SVG. `sketchScene` returns that drawing as a
scene to restyle, and `renderScene` draws a scene.

The package exports TypeScript types for every request and answer, generated from the engine's
JSON schema. The [guide](https://aarkue.github.io/graphlay/docs/guide) explains the options and the
[gallery](https://aarkue.github.io/graphlay/docs/gallery) shows what they do.
