// Copies the wasm-pack output next to the compiled TypeScript, which tsc does not do.
import { copyFileSync, mkdirSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const from = join(here, "..", "src", "pkg");
const to = join(here, "..", "dist", "pkg");
mkdirSync(to, { recursive: true });
for (const name of readdirSync(from)) {
  if (/\.(js|d\.ts|wasm)$/.test(name)) copyFileSync(join(from, name), join(to, name));
}
