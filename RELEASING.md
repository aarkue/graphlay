# Releasing

Pushing a tag `vX.Y.Z` releases everything. The `Release` workflow checks the version, runs the
tests, and publishes the crates, the npm package, the Python wheels, the command-line binaries and
a GitHub release with the changelog section.

## One-time setup

- crates.io, npm and PyPI all publish through trusted publishing. Each has a trusted publisher
  for owner `aarkue`, repository `graphlay` and workflow `release.yml`. PyPI's also names the
  environment `pypi`. No token secrets are needed.
- The repository needs a GitHub environment named `pypi`.
- GitHub Pages needs its source set to GitHub Actions in the repository's Pages settings. The
  `Site` workflow deploys on every push to `main`.

## Each release

1. Run `scripts/set-version.sh X.Y.Z`. It sets the version in `Cargo.toml`, in the CLI's
   dependency on `graphlay`, in `bindings/wasm/js/package.json` and in `Cargo.lock`, and adds a
   dated `## X.Y.Z` heading to the changelog. The Python package reads its version from
   `Cargo.toml`.
2. Write the changelog entries under the new heading.
3. Run the whole suite once more:

   ```sh
   cargo test --release --all-features
   cargo test -p graphlay-cli -p graphlay-wasm
   pnpm install && pnpm --filter graphlay build && pnpm --filter graphlay test
   pip install ./bindings/python && pytest bindings/python/tests
   ```

4. Commit, tag and push:

   ```sh
   git tag vX.Y.Z
   git push origin main vX.Y.Z
   ```

Before it publishes anything, the workflow stops if the tag differs from the manifest versions or
the changelog has no section for it.
