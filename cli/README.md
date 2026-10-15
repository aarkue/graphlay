# graphlay-cli

The [graphlay](https://github.com/aarkue/graphlay) layout engine on the command line. It reads
JSON and writes JSON or SVG, so any language that can start a process can use it.

```sh
cargo install graphlay-cli
echo '{"nodes": [{"size": {"width": 80, "height": 40}}, {"size": {"width": 80, "height": 40}}],
       "edges": [{"source": 0, "target": 1}]}' | graphlay layout --preset process_map --pretty
```

Add `--sketch` to `layout`, `reroute` or `tree` to get a plain SVG drawing instead of the layout.
To restyle that drawing, print it as a scene with `graphlay sketch --scene`, edit the JSON and draw
it with `graphlay svg`.

`graphlay --help` lists the subcommands: `layout`, `reroute`, `tree`, `metrics`, `sketch`,
`fingerprint`, `svg`, `presets`, `options` and `schema`. The JSON formats are those of the
`graphlay::json` module.
