// Gathers the content the site shows but does not own: the package READMEs, the changelog and
// the gallery drawings, which live next to the code they describe.
import { copyFileSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const site = join(dirname(fileURLToPath(import.meta.url)), "..");
const root = join(site, "..");

/** A README as a doc page: the H1 becomes the title and the rest is the body. */
function page(source, { title, slug, position, intro }) {
  let body = readFileSync(join(root, source), "utf8");
  const h1 = body.match(/^# (.+)\n/);
  if (h1) body = body.slice(h1[0].length);
  const front = [`title: ${JSON.stringify(title ?? h1?.[1] ?? source)}`];
  if (slug) front.push(`slug: ${slug}`);
  if (position) front.push(`sidebar_position: ${position}`);
  return `---\n${front.join("\n")}\n---\n\n${intro ?? ""}${body}`;
}

const generated = join(site, "generated");
rmSync(generated, { recursive: true, force: true });
mkdirSync(join(generated, "packages"), { recursive: true });
mkdirSync(join(generated, "changelog"), { recursive: true });

writeFileSync(
  join(generated, "packages", "javascript.md"),
  page("bindings/wasm/js/README.md", { title: "JavaScript", position: 1 }),
);
writeFileSync(
  join(generated, "packages", "python.md"),
  page("bindings/python/README.md", { title: "Python", position: 2 }),
);
writeFileSync(
  join(generated, "packages", "cli.md"),
  page("cli/README.md", { title: "Command line", position: 3 }),
);
writeFileSync(
  join(generated, "changelog", "changelog.md"),
  page("CHANGELOG.md", { title: "Changelog", slug: "/" }),
);

// The gallery drawings, for the landing page.
const gallery = join(site, "static", "gallery");
mkdirSync(gallery, { recursive: true });
for (const name of readdirSync(join(root, "docs", "gallery"))) {
  if (name.endsWith(".svg")) copyFileSync(join(root, "docs", "gallery", name), join(gallery, name));
}

// A landing page for the API reference, which the Pages workflow drops into static/api.
mkdirSync(join(site, "static", "api"), { recursive: true });
writeFileSync(
  join(site, "static", "api", "index.html"),
  '<!doctype html><meta http-equiv="refresh" content="0; url=graphlay/index.html">\n',
);
console.log("prepare: packages, changelog, gallery, api");
