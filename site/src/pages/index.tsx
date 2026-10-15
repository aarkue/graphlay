import Link from "@docusaurus/Link";
import useBaseUrl from "@docusaurus/useBaseUrl";
import Layout from "@theme/Layout";
import clsx from "clsx";
import type { ReactNode } from "react";

import styles from "./index.module.css";

const pictures = [
  ["process_models-3.svg", "An object-centric directly-follows graph", "/docs/gallery#process-models"],
  ["process_models-2.svg", "An object-centric Petri net", "/docs/gallery#process-models"],
  ["process_models-4.svg", "Road traffic fines, from a real log", "/docs/gallery#process-models"],
  ["styles-5.svg", "A service map", "/docs/gallery#drawing-styles"],
  ["styles-3.svg", "Frequencies as heat", "/docs/gallery#drawing-styles"],
  ["process_models-1.svg", "A Petri net: order to cash", "/docs/gallery#process-models"],
  ["merge_edges-2.svg", "Merged edges", "/docs/gallery#merged-edges"],
  ["swimlanes-2.svg", "Swimlanes", "/docs/gallery#swimlanes"],
  ["compounds-1.svg", "Nested subprocesses", "/docs/gallery#nested-subprocesses"],
  ["styles-4.svg", "A finite automaton", "/docs/gallery#drawing-styles"],
  ["wrapping.svg", "Wrapping into rows", "/docs/gallery#wrapping-into-rows"],
  ["port_hints-2.svg", "Port hints", "/docs/gallery#port-hints"],
] as const;

const installs = [
  ["Rust", "cargo add graphlay", "/docs/guide"],
  ["JavaScript", "npm install graphlay", "/packages/javascript"],
  ["Python", "pip install graphlay", "/packages/python"],
  ["Shell", "cargo install graphlay-cli", "/packages/cli"],
] as const;

const points: [string, string][] = [
  [
    "Deterministic",
    "The same graph and options give the same drawing on every platform and in every binding. A test suite pins the output of every example.",
  ],
  [
    "General purpose",
    "It lays out any directed graph. Process models are the main test case, so merged edges, swimlanes, port hints, nested subprocesses and row wrapping are built in.",
  ],
  [
    "Measured",
    "It computes crossings, bends, edges through nodes, staircase steps and more for any drawing. The test suite tracks them over a corpus of real models.",
  ],
  [
    "Interactive",
    "Drag a node and it re-routes only the edges. Seeds keep a relayout close to the drawing a person already knows.",
  ],
];

function Picture({ file, caption, to }: { file: string; caption: string; to: string }): ReactNode {
  return (
    <Link to={to} className={styles.picture}>
      <img src={useBaseUrl(`/gallery/${file}`)} alt={caption} loading="lazy" />
      <span>{caption}</span>
    </Link>
  );
}

export default function Home(): ReactNode {
  return (
    <Layout description="Deterministic layered graph layout in Rust, with packages for the browser and Python">
      <header className={clsx("hero", styles.hero)}>
        <div className="container">
          <h1 className="hero__title">graphlay</h1>
          <p className="hero__subtitle">
            Deterministic layered graph layout in Rust. One engine for Rust, the browser and Python.
          </p>
          <div className={styles.installs}>
            {installs.map(([name, command, to]) => (
              <Link key={name} to={to} className={styles.install}>
                <span className={styles.installName}>{name}</span>
                <code>{command}</code>
              </Link>
            ))}
          </div>
          <div className={styles.buttons}>
            <Link className="button button--primary button--lg" to="/demo">
              Try the demo
            </Link>
            <Link className="button button--secondary button--lg" to="/docs/guide">
              Read the guide
            </Link>
          </div>
        </div>
      </header>
      <main className="container">
        <section className={styles.pictures}>
          {pictures.map(([file, caption, to]) => (
            <Picture key={file} file={file} caption={caption} to={to} />
          ))}
        </section>
        <section className={styles.points}>
          {points.map(([title, text]) => (
            <div key={title} className={styles.point}>
              <h3>{title}</h3>
              <p>{text}</p>
            </div>
          ))}
        </section>
      </main>
    </Layout>
  );
}
