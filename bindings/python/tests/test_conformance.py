"""Every drawing in the conformance corpus, through this package, fingerprint for fingerprint."""

import json
from pathlib import Path

import pytest

import graphlay

ROOT = Path(__file__).resolve().parents[3] / "tests" / "conformance"
EXPECTED = json.loads((ROOT / "expected.json").read_text())


@pytest.mark.parametrize("key", sorted(EXPECTED))
def test_the_drawing_matches_the_core(key):
    kind, _ = key.split("/")
    request = json.loads((ROOT / "cases" / f"{key}.json").read_text())
    if kind == "tree":
        drawn = graphlay.layout_tree(request["graph"], options=request.get("options"))
    else:
        drawn = graphlay.layout(
            request["graph"], preset=request.get("preset", "layered"), options=request.get("options")
        )
    assert drawn.fingerprint() == EXPECTED[key]
