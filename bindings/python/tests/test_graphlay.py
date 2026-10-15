"""The package as a user calls it."""

import json

import pytest

import graphlay
from graphlay import Graph, GraphlayError, Layout

BOX = {"size": {"width": 80, "height": 40}}
TWO = {"nodes": [BOX, BOX], "edges": [{"source": 0, "target": 1}]}


def test_a_dict_graph_is_laid_out_left_to_right():
    out = graphlay.layout(TWO)
    assert isinstance(out, Layout)
    assert len(out.nodes) == 2 and len(out.edges) == 1
    assert out.nodes[0]["center"]["x"] < out.nodes[1]["center"]["x"]
    assert out.positions()[1] == (out.nodes[1]["center"]["x"], out.nodes[1]["center"]["y"])
    assert repr(out) == "Layout(2 nodes, 1 edges)"


def test_a_preset_and_an_options_patch_apply():
    out = graphlay.layout(TWO, preset="process_map", options={"routing": {"style": "diagonal"}})
    assert out.nodes[0]["center"]["y"] < out.nodes[1]["center"]["y"]
    assert graphlay.presets() == ["layered", "process_map", "hub_and_spoke", "hub_and_spoke_flow"]
    assert "spacing" in graphlay.options("hub_and_spoke")


def test_a_bad_option_raises_a_graphlay_error_naming_the_field():
    with pytest.raises(GraphlayError, match="^options.routing.style"):
        graphlay.layout(TWO, options={"routing": {"style": "curvy"}})
    with pytest.raises(GraphlayError):
        graphlay.layout({"nodes": [BOX], "edges": [{"source": 0, "target": 7}]})
    with pytest.raises(TypeError):
        graphlay.layout(42)


def test_a_graph_built_here_keeps_its_ids():
    g = Graph()
    g.node("start", width=60, height=30, shape="ellipse")
    g.node("work", label="Do the work")
    g.node("end", width=60, height=30, shape="ellipse")
    g.edge("start", "work")
    g.edge("work", "end", weight=2.0, thickness=3.0)
    out = graphlay.layout(g, preset="process_map")
    pos = out.positions(flip_y=True)
    assert set(pos) == {"start", "work", "end"}
    assert pos["start"][1] > pos["end"][1], "flipped y grows upwards"
    svg = out.sketch()
    assert "<svg" in svg and "Do the work" in svg and "start" in svg
    assert out._repr_svg_() == str(svg)
    with pytest.raises(ValueError):
        g.node("start")


def test_metrics_fingerprint_and_reroute():
    out = graphlay.layout(TWO)
    assert out.metrics()["crossings"] == 0
    assert len(out.fingerprint()) == 16
    assert graphlay.fingerprint(out) == out.fingerprint()
    moved = json.loads(json.dumps(TWO))
    for i, n in enumerate(out.nodes):
        moved["nodes"][i]["position"] = dict(n["center"])
    moved["nodes"][1]["position"]["x"] += 100
    again = graphlay.reroute(moved)
    assert again.nodes[1]["center"]["x"] == out.nodes[1]["center"]["x"] + 100


def test_a_tree_and_a_scene():
    tree = {"nodes": [BOX, BOX, BOX], "edges": [{"source": 0, "target": 1}, {"source": 0, "target": 2}]}
    out = graphlay.layout_tree(tree)
    assert out.nodes[0]["center"]["y"] < out.nodes[1]["center"]["y"]
    assert "<svg" in graphlay.render_scene({"nodes": [], "edges": []})
    scene = out.sketch_scene()
    scene["nodes"][0]["fill"] = "#ff0000"
    assert "#ff0000" in graphlay.render_scene(scene)


def test_a_networkx_graph_is_accepted():
    nx = pytest.importorskip("networkx")
    g = nx.DiGraph()
    g.add_node("a", label="Alpha", width=90)
    g.add_edge("a", "b", weight=2.0)
    g.add_edge("b", "c")
    out = graphlay.layout(g, preset="process_map")
    pos = out.positions()
    assert list(pos) == ["a", "b", "c"]
    assert pos["a"][1] < pos["c"][1]
    assert out.nodes[0]["size"]["width"] == 90
    assert "Alpha" in out.sketch()
