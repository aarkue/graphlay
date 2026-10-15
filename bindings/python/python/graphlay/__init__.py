"""Deterministic graph layout.

The engine is graphlay, a layered (Sugiyama-style) layout engine in Rust with orthogonal or
diagonal edge routing, a tidy-tree layout, quality metrics and an SVG renderer. A graph goes
in as a dict, a JSON string, a `Graph` built here, or a networkx graph, and a `Layout` comes
back with a position per node and a polyline per edge. In a notebook, a `Layout` displays
itself as a drawing.
"""

from __future__ import annotations

import json
from typing import Any, Dict, List, Mapping, Optional, Sequence, Tuple, Union

from . import _native
from ._native import GraphlayError

__all__ = [
    "Graph",
    "GraphlayError",
    "Layout",
    "Svg",
    "fingerprint",
    "layout",
    "layout_tree",
    "metrics",
    "options",
    "presets",
    "render_scene",
    "reroute",
    "sketch",
]

__version__ = _native.version()

GraphLike = Union[Mapping[str, Any], str, "Graph", Any]
"""A graph the functions here accept: a dict with `nodes` and `edges` as the engine's JSON
request spells them, that JSON as a string, a `Graph`, or a networkx graph."""


class Graph:
    """A graph to lay out, built node by node.

    Each node has a size in px and an id of your choice, and `layout().positions()` is keyed
    by those ids. Edges name their ends by id or by the index `node()` returned.
    """

    def __init__(self) -> None:
        self.nodes: List[Dict[str, Any]] = []
        self.edges: List[Dict[str, Any]] = []
        self.ids: List[Any] = []
        self._index: Dict[Any, int] = {}

    def node(
        self,
        id: Any = None,
        *,
        width: float = 120.0,
        height: float = 50.0,
        shape: str = "box",
        label: Optional[str] = None,
        **fields: Any,
    ) -> int:
        """Adds a node and returns its index. `fields` are further node fields of the
        engine's request, such as `rank`, `group`, `parent`, `position` or `pinned`."""
        index = len(self.nodes)
        if id is None:
            id = index
        if id in self._index:
            raise ValueError(f"node {id!r} was added twice")
        node = {"size": {"width": float(width), "height": float(height)}, "shape": shape}
        node["key"] = str(id) if label is None else label
        node.update(fields)
        self.nodes.append(node)
        self.ids.append(id)
        self._index[id] = index
        return index

    def edge(self, source: Any, target: Any, *, weight: float = 1.0, **fields: Any) -> int:
        """Adds an edge between two node ids (or indices) and returns its index. `fields` are
        further edge fields of the request, such as `thickness`, `label_size` or `group`."""
        edge = {"source": self._resolve(source), "target": self._resolve(target), "weight": weight}
        edge.update(fields)
        self.edges.append(edge)
        return len(self.edges) - 1

    def _resolve(self, ref: Any) -> int:
        if ref in self._index:
            return self._index[ref]
        if isinstance(ref, int) and 0 <= ref < len(self.nodes):
            return ref
        raise ValueError(f"no node {ref!r}")

    def to_dict(self) -> Dict[str, Any]:
        """The graph as the engine's request spells it."""
        return {"nodes": self.nodes, "edges": self.edges}

    @classmethod
    def from_networkx(
        cls,
        g: Any,
        *,
        width: float = 120.0,
        height: float = 50.0,
        shape: str = "box",
    ) -> "Graph":
        """A `Graph` from a networkx graph. Node attributes `width`, `height`, `shape` and
        `label` override the defaults, and the edge attributes `weight` and `thickness` pass
        through. An undirected graph is drawn with each edge pointing the way networkx lists
        it."""
        out = cls()
        for n, data in g.nodes(data=True):
            fields = {k: v for k, v in data.items() if k in ("rank", "group", "parent", "pinned")}
            out.node(
                n,
                width=data.get("width", width),
                height=data.get("height", height),
                shape=data.get("shape", shape),
                label=data.get("label"),
                **fields,
            )
        for u, v, data in g.edges(data=True):
            fields = {k: v for k, v in data.items() if k in ("thickness", "label_size", "group")}
            out.edge(u, v, weight=float(data.get("weight", 1.0)), **fields)
        return out


class Svg(str):
    """SVG markup. Displays as the drawing in a notebook."""

    def _repr_svg_(self) -> str:
        return str(self)

    def save(self, path: str) -> None:
        """Writes the markup to `path`."""
        with open(path, "w", encoding="utf-8") as f:
            f.write(self)


class Layout:
    """Where every node went and how every edge was drawn.

    `nodes[i]` and `edges[i]` follow the input order and are the engine's own dicts: a node
    has `center` and `size`, an edge has `points` from source to target, border to border.
    """

    def __init__(self, raw: Dict[str, Any], request: str, ids: Sequence[Any]) -> None:
        self.raw = raw
        self._request = request
        self.ids = list(ids)

    @property
    def nodes(self) -> List[Dict[str, Any]]:
        return self.raw["nodes"]

    @property
    def edges(self) -> List[Dict[str, Any]]:
        return self.raw["edges"]

    @property
    def bounds(self) -> Dict[str, Any]:
        """The drawing's bounding box: `min` and `max` points."""
        return self.raw["bounds"]

    def positions(self, *, flip_y: bool = False) -> Dict[Any, Tuple[float, float]]:
        """Node centres by node id, ready for `networkx.draw(g, pos=...)`. SVG y grows
        downwards, so pass `flip_y` for a plot whose y grows upwards."""
        sign = -1.0 if flip_y else 1.0
        return {
            id: (n["center"]["x"], sign * n["center"]["y"]) for id, n in zip(self.ids, self.nodes)
        }

    def to_dict(self) -> Dict[str, Any]:
        return self.raw

    def to_json(self) -> str:
        return json.dumps(self.raw)

    def fingerprint(self) -> str:
        """A stable fingerprint of the drawing, the same in every binding."""
        return _native.fingerprint(self.to_json())

    def metrics(self) -> Dict[str, Any]:
        """Quality metrics of the drawing: crossings, bends, node-edge hits and more."""
        return json.loads(_native.metrics(self._request, self.to_json()))

    def sketch(
        self,
        *,
        labels: Optional[Sequence[str]] = None,
        lane_names: Optional[Sequence[str]] = None,
        background: Optional[str] = "#ffffff",
        padding: float = 24.0,
    ) -> Svg:
        """A plain drawing: boxes with labels, edges with arrowheads, lanes behind."""
        opts = _sketch_options(labels, lane_names, background, padding)
        return Svg(_native.sketch(self._request, self.to_json(), opts))

    def sketch_scene(
        self,
        *,
        labels: Optional[Sequence[str]] = None,
        lane_names: Optional[Sequence[str]] = None,
        background: Optional[str] = "#ffffff",
        padding: float = 24.0,
    ) -> Dict[str, Any]:
        """The plain drawing as a scene dict, to restyle and pass to `render_scene`."""
        opts = _sketch_options(labels, lane_names, background, padding)
        return json.loads(_native.sketch_scene(self._request, self.to_json(), opts))

    def _repr_svg_(self) -> str:
        return str(self.sketch())

    def __repr__(self) -> str:
        return f"Layout({len(self.nodes)} nodes, {len(self.edges)} edges)"


def _sketch_options(
    labels: Optional[Sequence[str]],
    lane_names: Optional[Sequence[str]],
    background: Optional[str],
    padding: float,
) -> str:
    return json.dumps(
        {
            "labels": list(labels or []),
            "lane_names": list(lane_names or []),
            "background": background,
            "padding": padding,
        }
    )


def _graph_dict(graph: GraphLike) -> Tuple[Dict[str, Any], List[Any]]:
    """The engine's graph dict and the node ids, from any accepted input."""
    if isinstance(graph, Graph):
        return graph.to_dict(), graph.ids
    if isinstance(graph, str):
        graph = json.loads(graph)
    if isinstance(graph, Mapping):
        if "graph" in graph and "nodes" not in graph:
            graph = graph["graph"]
        nodes = graph.get("nodes", [])
        return dict(graph), list(range(len(nodes)))
    if hasattr(graph, "nodes") and hasattr(graph, "edges") and callable(graph.edges):
        built = Graph.from_networkx(graph)
        return built.to_dict(), built.ids
    raise TypeError(f"not a graph: {type(graph).__name__}")


def _request(graph: GraphLike, preset: Optional[str], options: Optional[Mapping[str, Any]]) -> Tuple[str, List[Any]]:
    g, ids = _graph_dict(graph)
    request: Dict[str, Any] = {"graph": g}
    if preset is not None:
        request["preset"] = preset
    if options:
        request["options"] = dict(options)
    return json.dumps(request), ids


def layout(
    graph: GraphLike,
    *,
    preset: str = "layered",
    options: Optional[Mapping[str, Any]] = None,
) -> Layout:
    """Lays out `graph` with the layered engine.

    `preset` is one of `presets()`. `options` is a patch over the preset's options, such as
    `{"direction": "topbottom", "routing": {"style": "diagonal"}}`.
    """
    request, ids = _request(graph, preset, options)
    return Layout(json.loads(_native.layout(request)), request, ids)


def reroute(
    graph: GraphLike,
    *,
    preset: str = "layered",
    options: Optional[Mapping[str, Any]] = None,
) -> Layout:
    """Re-routes the edges over the `position` each node carries, without moving a node.
    For a drag, give every node its current centre and the dragged one its new centre."""
    request, ids = _request(graph, preset, options)
    return Layout(json.loads(_native.reroute(request)), request, ids)


def layout_tree(graph: GraphLike, *, options: Optional[Mapping[str, Any]] = None) -> Layout:
    """Lays out a rooted tree or forest with the tidy-tree engine, parents centred over
    their children."""
    request, ids = _request(graph, None, options)
    return Layout(json.loads(_native.layout_tree(request)), request, ids)


def metrics(layout: Layout) -> Dict[str, Any]:
    """Quality metrics of a layout, the same as `layout.metrics()`."""
    return layout.metrics()


def fingerprint(layout: Layout) -> str:
    """A stable fingerprint of a layout, the same as `layout.fingerprint()`."""
    return layout.fingerprint()


def sketch(layout: Layout, **kwargs: Any) -> Svg:
    """A plain drawing of a layout, the same as `layout.sketch(...)`."""
    return layout.sketch(**kwargs)


def presets() -> List[str]:
    """The presets a layout can start from."""
    return list(_native.presets())


def options(preset: str = "layered") -> Dict[str, Any]:
    """The full options of a preset, to see or edit every field."""
    return json.loads(_native.options(preset))


def render_scene(scene: Mapping[str, Any], palette: Optional[Mapping[str, Any]] = None) -> Svg:
    """Draws a scene (the engine's `svg::Scene`, e.g., from `Layout.sketch_scene`) to SVG.
    `palette` holds the fallback colours, and `None` means the default ones."""
    return Svg(_native.render_scene(json.dumps(scene), json.dumps(palette) if palette else ""))
