import BrowserOnly from "@docusaurus/BrowserOnly";
import Layout from "@theme/Layout";
import type { ReactNode } from "react";

// The engine is WebAssembly, so the demo renders in the browser only.
export default function DemoPage(): ReactNode {
  return (
    <Layout title="Demo" description="Lay out a graph in the browser, drag nodes, and read the metrics" noFooter>
      <BrowserOnly fallback={<p style={{ padding: "1rem" }}>Loading the demo...</p>}>
        {() => {
          const Demo = require("../components/demo/Demo").default;
          return <Demo />;
        }}
      </BrowserOnly>
    </Layout>
  );
}
