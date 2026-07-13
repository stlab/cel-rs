[![CI](https://github.com/stlab/cel-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/stlab/cel-rs/actions/workflows/ci.yml)

## begin: Spectrum Web Components bundle

`begin/assets/swc.js` is a single bundle (elements + Spectrum 2 theme tokens + the
zoom-control icons) produced by esbuild from real npm packages. It's committed like
every other vendored asset, so cloning and building `begin` needs no Node/npm setup.

Node.js + npm are only needed if you're updating the Spectrum version or otherwise
need to regenerate `begin/assets/swc.js`:

```bash
cargo xtask build-js
```

or directly:

```bash
cd begin
npm ci
npm run build
```

Commit the regenerated `begin/assets/swc.js` along with any `begin/package.json`/
`begin/package-lock.json` changes. See
[docs/superpowers/specs/2026-07-11-begin-spectrum2-theme-tokens-design.md](docs/superpowers/specs/2026-07-11-begin-spectrum2-theme-tokens-design.md)
for why this needs to be one compiled bundle rather than separate vendored/live files.
