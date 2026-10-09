# Adam slides

`adam-slides` is a Marp presentation with live Adam examples. Its initial examples
come from the Adam language book's tutorial, but the deck can mix text slides
with any supported book example or deck-local `.adm2` source. Example panes display
the source alongside its live inspector and an optional graph.
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
directives from the staged book manifest and deck-local `.adm2` files, and converts
the Markdown with Marp.
Missing assets or example references fail the build.
The generator stages assets privately and validates sources before updating
`dist`, so an invalid edit leaves the last valid deck and manifest available.

## Author slides

Edit `slides.md` and separate slides with `---`. Text-only slides need no example
directive. Add, remove, repeat, or reorder examples independently of the tutorial:

```markdown
<!-- adam-example: tutorial/first_sheet graph=1 -->
```

Book keys are paths beneath `adam-lang-book/book-src/examples`, without `.adm2`.
Any source in the staged book manifest is available, not just tutorial sources.
Examples intentionally excluded from live mounting, such as
`expressions/no_standard_library`, fail the build when referenced.

For a new deck-local example, create `adam-slides/examples/custom.adm2` and use:

```markdown
<!-- adam-example: local/custom -->
```

Nested files work too: `examples/demo/custom.adm2` uses `local/demo/custom`.
Each path component must start with a lowercase letter and contain only lowercase
letters, digits, underscores, or hyphens. `local/` is reserved for deck-local sources.
The generator embeds exactly the source that the live inspector loads.

Graphs default to off. Use `graph=1` to enable one, or `graph=0` to disable it
explicitly. Each directive creates an independent live example; slides can contain
multiple directives, but their panes share the available space. Keep prose and
example count small enough to fit a slide. Adjust layout in `slides.css`.

## Preview and verify

Run these commands from `adam-slides`:

```powershell
npm run preview
```

Open `http://127.0.0.1:3419/project/docs/adam-slides/`. This optional preview
server deliberately uses a nested URL to exercise GitHub Pages-style paths.
Stop it with Ctrl+C. Live modules need HTTP hosting rather than opening
`dist\index.html` as a `file:` URL.

### Live preview

In VS Code, choose **Terminal > Run Task > slides: serve (full)**. The task builds
the shared WASM runtime, runs `npm ci`, builds the deck, and starts the source
watcher, Marp watcher, and preview server in dedicated terminals. Open
`http://127.0.0.1:3419/project/docs/adam-slides/` after the server reports that URL.
No initial refresh is needed: the task starts Marp's watched build before serving.

Saves to `slides.md`, book example sources, or deck-local `.adm2` files regenerate
the content and automatically refresh the browser. Marp also watches `slides.css`.
Changes to the example host's HTML, JavaScript, and CSS trigger regeneration.
Each refresh resets the live examples' edited values. Use **Terminal > Terminate
Task** to stop all three running `slides:` tasks; stopping only the compound task
does not necessarily stop its watchers.

For manual startup, run `npm run build` first, then open three terminals in
`adam-slides`. Start these commands in order, waiting for source generation and
Marp's first watched conversion before starting the server:

```powershell
npm run watch:sources
```

```powershell
npm run watch:marp
```

```powershell
npm run preview
```

Open the URL after all three are ready. If the page was already open before
starting Marp watch, refresh it once to load the live-reload connection. Stop each
process with Ctrl+C. The source watcher uses Node's `--watch-path`, supported on
Windows and macOS; static builds and browser checks also run on Linux.

### Automated checks

For automated checks, stop the preview server and both watchers first:

```powershell
npm test
npx playwright install chromium
npm run test:browser
```

Browser tests use a separate acceptance deck with text slides, reordered examples,
additional prose, and a local source, so editing the authored deck does not remove
regression coverage. They also check every selected mount in the actual deck.
Coverage includes canonical source, value edits, graph
direction, conditional activation, forced values, diagnostics, input isolation,
preserved state, resizing, and visible startup failures, including a missing
bootstrap and dependencies that load but cannot execute or register. They also capture slide
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
