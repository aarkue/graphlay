# Releasing

Pushing a tag `vX.Y.Z` releases everything. The `Release` workflow checks the version, runs the
tests, and publishes the crates, the npm package, the Python wheels, the command-line binaries and
a GitHub release with the changelog section.

## One-time setup

- crates.io needs an API token with publish rights for `graphlay` and `graphlay-cli`, stored as
  the `CARGO_REGISTRY_TOKEN` repository secret. After the first release, trusted publishing can
  replace it.
- npm needs an automation token for the `graphlay` package, stored as `NPM_TOKEN`. The first
  publish creates the package. After it, a trusted publisher on npmjs.com for this repository and
  the `release.yml` workflow can replace the token.
- PyPI needs a pending trusted publisher for the project `graphlay` with this repository, the
  workflow `release.yml` and the environment `pypi`. Create the `pypi` environment in the
  repository settings as well. No token is needed.
- GitHub Pages needs its source set to GitHub Actions in the repository's Pages settings. The
  `Site` workflow deploys on every push to `main`.

## Each release

1. Set the version in `Cargo.toml` under `[workspace.package]` and in
   `bindings/wasm/js/package.json`. The Python package reads it from `Cargo.toml`.
2. Turn the changelog's `## x.y.z (unreleased)` heading into `## x.y.z (YYYY-MM-DD)`.
3. Run the whole suite once more:

   ```sh
   cargo test --release --all-features
   cargo test -p graphlay-cli -p graphlay-wasm
   pnpm install && pnpm --filter graphlay build && pnpm --filter graphlay test
   pip install ./bindings/python && pytest bindings/python/tests
   ```

4. Commit, tag and push:

   ```sh
   git tag v0.2.0
   git push origin main v0.2.0
   ```

Before it publishes anything, the workflow stops if the tag differs from the manifest versions or
the changelog has no section for it.
