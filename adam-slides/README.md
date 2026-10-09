# Adam slides

`adam-slides` is a Marp presentation with the nine visible examples from the
Adam language book's tutorial. Each slide displays the canonical Adam source
alongside its live inspector and, where the tutorial includes one, its graph.
Edit cell values to explore propagation, constraints, and diagnostics.

The output is a **static website**. JavaScript and WebAssembly run Adam in the
browser; no application server, published book, or runtime CDN is required.

## Build

Prerequisites: Rust with the `wasm32-unknown-unknown` target, `wasm-pack`, and
Node.js 22 or newer. The repository already vendors the Spectrum and graph
assets; do not rebuild those bundles unless changing their inputs.

From the repository root in PowerShell:

```powershell
rustup target add wasm32-unknown-unknown
cargo install wasm-pack --locked
wasm-pack build --target web --release .\adam-lang-book-live
Set-Location adam-slides
npm ci
npm run build
```

The build stages shared runtime assets through
`cargo run -p xtask -- prepare-live-slides-assets`, resolves the deck's example
directives from the book's `.adm2` files, and converts the Markdown with Marp.
Missing assets or example references fail the build.

Edit titles and interaction prompts in `slides.md`, and layout in `slides.css`.
Each slide has one `<!-- adam-example: tutorial/name -->` directive. The build
checks that the ordered directives match the tutorial's active example includes;
commented-out tutorial sections do not create slides.

## Preview and verify

Run these commands from `adam-slides`:

```powershell
npm run preview
```

Open `http://127.0.0.1:3419/project/docs/adam-slides/`. This optional preview
server deliberately uses a nested URL to exercise GitHub Pages-style paths.
Stop it with Ctrl+C. Live modules need HTTP hosting rather than opening
`dist\index.html` as a `file:` URL.

For automated checks, stop the preview server first:

```powershell
npm test
npx playwright install chromium
npm run test:browser
```

Browser tests cover all nine mounts, canonical source, value edits, graph
direction, conditional activation, forced values, diagnostics, input isolation,
preserved state, resizing, and visible startup failures. They also capture slide
screenshots under the ignored `test-results` directory.

Click the slide title or another area outside the live frame before using Marp's
keyboard navigation. Keyboard events inside examples stay with their controls.
Marp's presentation controls also navigate slides without discarding live edits.

## Publish

Publish the **contents** of `dist` together, including `theme`, `example.html`,
`example.mjs`, and `example.css`. The directory can be hosted at the site root
or beneath a project/documentation path without changing asset URLs.

The existing Docs workflow builds and verifies the deck, then includes it in
the Pages artifact at `target/doc/adam-slides`. Once that workflow deploys this
branch's changes, the deck is available beneath the documentation site's
`adam-slides/` URL. No separate deployment or backend is needed.

Only HTML retains the live examples. PDF and PowerPoint exports are not
interactive presentation formats for this deck.

## Build dependency maintenance

The lockfile includes compatible upstream fixes, and `basic-ftp` is pinned to
patched version 6.2.2 through an override. Marp's remaining build-time
`extract-zip` and KaTeX advisories are tracked in
[issue #249](https://github.com/stlab/cel-rs/issues/249); `npm audit` does not
currently pass. This build converts trusted repository Markdown to HTML rather
than downloading browsers, extracting archives, or rendering authored math.
None of the Node dependencies ship in the static site. Update the supported
Marp toolchain when upstream fixes become available rather than apply npm's
suggested downgrade to an obsolete Marp release.
