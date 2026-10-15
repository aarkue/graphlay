import type * as Preset from "@docusaurus/preset-classic";
import type { Config } from "@docusaurus/types";
import { themes as prismThemes } from "prism-react-renderer";

const config: Config = {
  title: "graphlay",
  tagline: "Deterministic graph layout in Rust, for Rust, the browser and Python",
  favicon: "img/favicon.svg",
  url: "https://aarkue.github.io",
  baseUrl: process.env.SITE_BASE ?? "/graphlay/",
  organizationName: "aarkue",
  projectName: "graphlay",
  trailingSlash: false,
  onBrokenLinks: "throw",
  onBrokenAnchors: "throw",
  markdown: {
    // The docs are plain Markdown shared with GitHub, not MDX.
    format: "detect",
    hooks: { onBrokenMarkdownLinks: "throw" },
  },
  i18n: { defaultLocale: "en", locales: ["en"] },
  presets: [
    [
      "classic",
      {
        docs: {
          path: "../docs",
          routeBasePath: "docs",
          sidebarPath: "./sidebars.ts",
          editUrl: "https://github.com/aarkue/graphlay/edit/main/",
        },
        blog: false,
        theme: { customCss: "./src/css/custom.css" },
      } satisfies Preset.Options,
    ],
  ],
  plugins: [
    [
      "@docusaurus/plugin-content-docs",
      {
        id: "packages",
        path: "generated/packages",
        routeBasePath: "packages",
        sidebarPath: "./sidebars-packages.ts",
      },
    ],
    [
      "@docusaurus/plugin-content-docs",
      {
        id: "changelog",
        path: "generated/changelog",
        routeBasePath: "changelog",
        sidebarPath: false,
      },
    ],
  ],
  themes: [
    [
      "@easyops-cn/docusaurus-search-local",
      {
        hashed: true,
        docsRouteBasePath: ["docs", "packages"],
        docsDir: ["../docs", "generated/packages"],
        indexBlog: false,
        indexPages: true,
      },
    ],
  ],
  themeConfig: {
    colorMode: { respectPrefersColorScheme: true },
    navbar: {
      title: "graphlay",
      logo: { alt: "", src: "img/logo.svg" },
      items: [
        { to: "/docs/guide", label: "Guide", position: "left" },
        { to: "/docs/gallery", label: "Gallery", position: "left" },
        { to: "/demo", label: "Demo", position: "left" },
        { to: "/packages/javascript", label: "Packages", position: "left" },
        { href: "pathname:///graphlay/api/graphlay/", label: "API", position: "left" },
        { to: "/changelog", label: "Changelog", position: "right" },
        { href: "https://github.com/aarkue/graphlay", label: "GitHub", position: "right" },
      ],
    },
    footer: {
      style: "dark",
      links: [
        {
          title: "Use it",
          items: [
            { label: "Rust crate", href: "https://crates.io/crates/graphlay" },
            { label: "npm package", href: "https://www.npmjs.com/package/graphlay" },
            { label: "Python package", href: "https://pypi.org/project/graphlay/" },
          ],
        },
        {
          title: "Read",
          items: [
            { label: "Guide", to: "/docs/guide" },
            { label: "Quality metrics", to: "/docs/quality" },
            { label: "Architecture", to: "/docs/architecture" },
          ],
        },
        {
          title: "Around it",
          items: [
            { label: "rust4pm", href: "https://github.com/aarkue/rust4pm" },
            { label: "propel", href: "https://github.com/aarkue/propel" },
          ],
        },
      ],
      copyright: "graphlay is MIT or Apache-2.0 licensed.",
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
      additionalLanguages: ["rust", "toml", "python", "bash", "json"],
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
