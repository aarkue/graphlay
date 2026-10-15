// The graphlay engine for JavaScript. The engine is Rust compiled to WebAssembly and embedded in
// this package. This file loads it once and gives every entry point a typed signature.

import type * as T from "./types.js";

export type {
  Edge,
  EdgeRoute,
  Graph,
  Lane,
  Layout,
  LayoutMetrics,
  LayoutOptions,
  LayoutRequest as RawLayoutRequest,
  Node,
  Palette,
  Placement,
  Point,
  Preset,
  Rect,
  Scene,
  Size,
  SketchOptions,
  TreeOptions,
  TreeRequest as RawTreeRequest,
} from "./types.js";

type Native = typeof import("./pkg/graphlay.js");

/** Every field optional, recursively: the shape of an options patch. */
export type DeepPartial<X> = X extends object ? { [K in keyof X]?: DeepPartial<X[K]> } : X;

/** A graph, the preset to start from and the option fields to change. */
export interface LayoutRequest extends Omit<T.LayoutRequest, "options"> {
  options?: DeepPartial<T.LayoutOptions> | null;
}

/** A rooted tree or forest and the tree option fields to change. */
export interface TreeRequest extends Omit<T.TreeRequest, "options"> {
  options?: DeepPartial<T.TreeOptions> | null;
}

/** Where the wasm loads from. The embedded bytes are the default. */
export type WasmSource =
  | string
  | URL
  | Request
  | Response
  | Promise<Response>
  | BufferSource
  | WebAssembly.Module;

/** An error the engine reported: a bad request, an invalid graph or a limit exceeded. */
export class GraphlayError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "GraphlayError";
  }
}

/** The loaded engine. Every call is synchronous. */
export interface Engine {
  /** The engine's version. */
  readonly version: string;
  /** Lays out a graph with the layered engine. */
  layout(request: LayoutRequest): T.Layout;
  /** Re-routes the edges over the positions the nodes carry, for a drag without a relayout. */
  reroute(request: LayoutRequest): T.Layout;
  /** Lays out a rooted tree or forest with the tidy-tree engine. */
  layoutTree(request: TreeRequest): T.Layout;
  /** Measures a layout against the request that produced it. */
  metrics(request: LayoutRequest, layout: T.Layout): T.LayoutMetrics;
  /** A stable fingerprint of a layout, the same in every binding. */
  fingerprint(layout: T.Layout): string;
  /** The presets a request can start from. */
  presets(): T.Preset[];
  /** The full options of a preset. */
  options(preset: T.Preset): T.LayoutOptions;
  /** Draws a scene to SVG markup. The palette holds the fallback colours. */
  renderScene(scene: DeepPartial<T.Scene>, palette?: T.Palette): string;
  /** Draws a layout of the request's graph as a plain sketch, with node keys as labels. */
  sketch(request: LayoutRequest | TreeRequest, layout: T.Layout, options?: Partial<T.SketchOptions>): string;
  /** The plain sketch as a scene, to restyle before `renderScene`. */
  sketchScene(
    request: LayoutRequest | TreeRequest,
    layout: T.Layout,
    options?: Partial<T.SketchOptions>,
  ): T.Scene;
}

let loading: Promise<Engine> | undefined;

function call<R>(f: () => R): R {
  try {
    return f();
  } catch (e) {
    throw new GraphlayError(typeof e === "string" ? e : e instanceof Error ? e.message : String(e));
  }
}

function wrap(native: Native): Engine {
  const json = <R>(text: string): R => JSON.parse(text) as R;
  return {
    version: native.version(),
    layout: (request) => json(call(() => native.layout(JSON.stringify(request)))),
    reroute: (request) => json(call(() => native.reroute(JSON.stringify(request)))),
    layoutTree: (request) => json(call(() => native.layout_tree(JSON.stringify(request)))),
    metrics: (request, layout) =>
      json(call(() => native.metrics(JSON.stringify(request), JSON.stringify(layout)))),
    fingerprint: (layout) => call(() => native.fingerprint(JSON.stringify(layout))),
    presets: () => json(native.presets()),
    options: (preset) => json(call(() => native.options(preset))),
    renderScene: (scene, palette) =>
      call(() => native.render_scene(JSON.stringify(scene), palette ? JSON.stringify(palette) : "")),
    sketch: (request, layout, options) =>
      call(() =>
        native.sketch(
          JSON.stringify(request),
          JSON.stringify(layout),
          options ? JSON.stringify(options) : "",
        ),
      ),
    sketchScene: (request, layout, options) =>
      json(
        call(() =>
          native.sketch_scene(
            JSON.stringify(request),
            JSON.stringify(layout),
            options ? JSON.stringify(options) : "",
          ),
        ),
      ),
  };
}

async function embeddedBytes(): Promise<Uint8Array> {
  const { wasmBase64 } = await import("./pkg/graphlay_inline.js");
  const g = globalThis as {
    atob?: (s: string) => string;
    Buffer?: { from(s: string, encoding: string): Uint8Array };
  };
  if (typeof g.atob === "function") {
    return Uint8Array.from(g.atob(wasmBase64), (c) => c.charCodeAt(0));
  }
  if (g.Buffer) return new Uint8Array(g.Buffer.from(wasmBase64, "base64"));
  throw new GraphlayError("no base64 decoder (atob or Buffer) to unpack the embedded wasm");
}

/**
 * Loads the engine once and returns it. Later calls return the same engine. Pass a source to
 * load the wasm from a URL or bytes of your own instead of the embedded copy. A source given
 * after the first load is ignored.
 */
export function load(source?: WasmSource): Promise<Engine> {
  if (!loading) {
    loading = (async () => {
      try {
        const native = await import("./pkg/graphlay.js");
        await native.default({ module_or_path: source ?? (await embeddedBytes()) });
        return wrap(native);
      } catch (e) {
        loading = undefined;
        throw e instanceof GraphlayError
          ? e
          : new GraphlayError(`the engine failed to load: ${e instanceof Error ? e.message : String(e)}`);
      }
    })();
  }
  return loading;
}

/** Lays out a graph with the layered engine, loading the engine on first use. */
export async function layout(request: LayoutRequest): Promise<T.Layout> {
  return (await load()).layout(request);
}

/** Re-routes the edges over the positions the nodes carry. */
export async function reroute(request: LayoutRequest): Promise<T.Layout> {
  return (await load()).reroute(request);
}

/** Lays out a rooted tree or forest with the tidy-tree engine. */
export async function layoutTree(request: TreeRequest): Promise<T.Layout> {
  return (await load()).layoutTree(request);
}

/** Measures a layout against the request that produced it. */
export async function metrics(request: LayoutRequest, layout: T.Layout): Promise<T.LayoutMetrics> {
  return (await load()).metrics(request, layout);
}

/** A stable fingerprint of a layout. */
export async function fingerprint(layout: T.Layout): Promise<string> {
  return (await load()).fingerprint(layout);
}

/** Draws a scene to SVG markup. */
export async function renderScene(scene: DeepPartial<T.Scene>, palette?: T.Palette): Promise<string> {
  return (await load()).renderScene(scene, palette);
}

/** Draws a layout of the request's graph as a plain sketch. */
export async function sketch(
  request: LayoutRequest | TreeRequest,
  layout: T.Layout,
  options?: Partial<T.SketchOptions>,
): Promise<string> {
  return (await load()).sketch(request, layout, options);
}

/** The plain sketch as a scene, to restyle before `renderScene`. */
export async function sketchScene(
  request: LayoutRequest | TreeRequest,
  layout: T.Layout,
  options?: Partial<T.SketchOptions>,
): Promise<T.Scene> {
  return (await load()).sketchScene(request, layout, options);
}
