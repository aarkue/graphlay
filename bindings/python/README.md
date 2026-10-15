# graphlay

Deterministic graph layout for Python: a layered (Sugiyama-style) engine with orthogonal or
diagonal edge routing, a tidy-tree layout, quality metrics and an SVG renderer. The engine is
[graphlay](https://github.com/aarkue/graphlay), written in Rust. The wheels are prebuilt, so there
is nothing to compile.

```sh
pip install graphlay
```

```python
import graphlay

g = graphlay.Graph()
g.node("start", width=60, height=30, shape="ellipse")
g.node("work", label="Do the work")
g.node("end", width=60, height=30, shape="ellipse")
g.edge("start", "work")
g.edge("work", "end")

drawing = graphlay.layout(g, preset="process_map", options={"routing": {"style": "diagonal"}})
drawing.positions()   # {"start": (x, y), ...}
drawing.edges[0]["points"]   # the polyline, border to border
drawing                      # in a notebook: the drawing itself
drawing.sketch().save("drawing.svg")
```

A networkx graph works directly, and `positions()` feeds `networkx.draw`:

```python
import networkx as nx

g = nx.DiGraph([("a", "b"), ("b", "c"), ("a", "c")])
pos = graphlay.layout(g).positions(flip_y=True)
nx.draw(g, pos)
```

`drawing.sketch_scene()` returns the same drawing as a scene dict. Change colours or labels in
it and draw it with `graphlay.render_scene(scene)`.

The [guide](https://aarkue.github.io/graphlay/docs/guide) explains the options and the
[gallery](https://aarkue.github.io/graphlay/docs/gallery) shows what they do.
