// Every drawing in the conformance corpus, through this package, fingerprint for fingerprint.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

import { load } from "../dist/index.js";

const root = new URL("../../../../tests/conformance/", import.meta.url);
const expected = JSON.parse(readFileSync(new URL("expected.json", root), "utf8"));
const engine = await load();

for (const [key, fingerprint] of Object.entries(expected)) {
  test(key, () => {
    const [kind] = key.split("/");
    const request = JSON.parse(readFileSync(new URL(`cases/${key}.json`, root), "utf8"));
    const drawn = kind === "tree" ? engine.layoutTree(request) : engine.layout(request);
    assert.equal(engine.fingerprint(drawn), fingerprint);
  });
}
