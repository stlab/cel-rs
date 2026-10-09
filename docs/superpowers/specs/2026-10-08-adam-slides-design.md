# Adam tutorial slides

## Intent and approved scope

Create `adam-slides`, a Marp presentation containing one slide for each visible
example in the Adam language book's tutorial. Preserve the tutorial's live
behavior: viewers edit cell values and observe constraint propagation,
diagnostics, and graph changes. Source code is displayed, not edited.

The deliverable is a self-contained static website suitable for GitHub Pages.
Adam executes entirely in the browser through the existing WebAssembly runtime.
There is no application server, runtime CDN dependency, or dependency on the
published book. Local HTTP serving is an optional preview mechanism, not a
deployment requirement.

The user approved a Marp deck with embedded static example pages after clarifying
that the pages and their assets reside in the same static site.

## Existing integration

- `adam-lang-book/book-src/tutorial.md:11-14` defines the tutorial's live
  experience. Its visible example includes occupy lines 21-151. Lines 154-207
  comment out additional examples; those examples are outside this initial deck.
- `adam-lang-book-live/src/lib.rs:120-139` exports `mount`, which accepts an
  inspector container, Adam source, diagnostic name, and graph container IDs.
- `adam-lang-book-live/src/lib.rs:57-99` connects the inspector and its graphs to
  the same sheet instance and presents sheet diagnostics.
- `adam-lang-book/book-src/theme/adam-live-bootstrap.js:48-74` loads Spectrum,
  WebAssembly, the source manifest, and optional graph dependencies before
  mounting examples.
- `xtask/src/live_book_assets.rs:108-154` generates the source manifest and stages
  the existing Spectrum, inspector, graph, D3, and WebAssembly assets.
- `.github/workflows/docs.yml:51-63` already builds the live WebAssembly runtime
  and the book. Lines 84-87 publish `target/doc` through GitHub Pages.

Reuse these producers and the existing mount interface instead of implementing
a second Adam runtime or inspector.

## Content

The initial deck has exactly nine example slides, in the tutorial's order:

| Example key | Slide title | Graph |
| --- | --- | --- |
| `tutorial/first_sheet` | A first sheet | Yes |
| `tutorial/clamp_demo` | Filters | No |
| `tutorial/basic_output` | Out cells | Yes |
| `tutorial/basic_relationship` | Cells and relationships | Yes |
| `tutorial/inequality` | Chained relationships | Yes |
| `tutorial/constrain` | Conditionals | Yes |
| `tutorial/conditional_forced` | Forced conditional values | No |
| `tutorial/requirements_filter_diagnostic` | Filter diagnostics | No |
| `tutorial/area_with_requirement` | Requirements | No |

Each slide contains a title, its complete canonical `.adm2` source, and a live
example. A short interaction prompt may explain the behavior to demonstrate.
Do not add a separate title slide or include examples from the commented-out
tutorial section.

Read source from `adam-lang-book/book-src/examples/tutorial/` during the build.
Do not commit duplicate Adam sheets. Keep slide titles and prompts editable in
the deck's Markdown. Validate its ordered example references against the active
tutorial includes so content cannot silently drift.

## Architecture

### Marp presentation

Use Marp's HTML presentation output and built-in presentation navigation.
Enable raw HTML only for the repository-authored deck so slides can contain
iframes. Do not accept untrusted Markdown or inject arbitrary example source as
HTML.

Each slide embeds a same-site static example host page in an iframe. The frame
isolates Spectrum and inspector styling from Marp's slide CSS and keeps input
events inside the example from reaching presentation shortcuts.

Display source next to the live example. Use a consistent widescreen layout with
bounded panes: long source can scroll, and the inspector and graph remain usable
without expanding the slide. Use the tutorial's graph presence rather than
adding graphs to every slide.

### Static example host

Provide one reusable host page selected by an example key and graph setting.
The host loads the staged source manifest, Spectrum, and WebAssembly; when a
graph is requested, it also loads D3 and the graph driver before mounting.
Its inspector and graph use one sheet instance, as in the tutorial.

Resolve asset URLs relative to the host page or its script, not to a hard-coded
domain or root path. The same files must work below GitHub Pages project paths
and nested documentation paths.

Validate the example selector and graph setting. Show a visible loading state,
then either the live example or a useful failure message. Missing examples,
failed HTTP responses, failed module/script loads, and WebAssembly initialization
errors must not leave an unexplained blank frame. Preserve the runtime's own
parse and propagation diagnostics.

Keep each frame mounted after it first loads so navigating away and back
preserves edits. Do not rebuild a sheet on each slide activation. Size changes
and previously hidden slides must not leave graphs blank or mis-sized; verify
the host and graph resize behavior in the browser and implement the necessary
activation or resize handling.

### Build and assets

Proposed project layout:

- `adam-slides/`: authored Marp Markdown, presentation theme, static host, and
  focused JavaScript build/runtime tests.
- `adam-slides/package.json` and its lockfile: pinned Marp build dependencies
  and reproducible build/test commands.
- `adam-slides/dist/`: generated static output, ignored by Git.

The build resolves the Markdown's example references to canonical source and
renders the deck through Marp. It packages the host and all required runtime
assets into `dist`; generated files are not committed.

Reuse the existing live-asset staging implementation. If it needs another output
destination, parameterize the shared staging operation while preserving the
existing book command's destination and behavior. Do not duplicate its asset
list or WebAssembly-copy implementation.

Build-time tooling may require Node, Rust, and wasm-pack. The delivered site
requires only a browser and a static HTTP host. Document the exact clean-checkout
build and preview commands, including runtime asset prerequisites.

## Publishing

Integrate the slide build into the existing documentation workflow, reusing its
WebAssembly build. Copy the completed static output into `target/doc/adam-slides`
before uploading the Pages artifact. Preserve the existing books and rustdoc.

The deployed entry point is the documentation site's `adam-slides/` directory.
The standalone output can also be deployed unchanged as another static site's
root. Do not introduce a second Pages deployment workflow or change the existing
deployment permissions.

## Interaction contract

- Writable cell controls preserve the tutorial's write/propagate behavior.
- Forced outputs remain non-editable; diagnostics remain visible.
- Graphs update from the inspector's sheet, including relationship direction and
  conditional activation changes.
- Keyboard input, sliders, graph interactions, and scrolling inside a frame do
  not advance the presentation.
- Presentation navigation remains available outside the frames.
- Returning to a visited slide preserves its live values.
- Every slide shows complete source and usable controls at the presentation's
  standard widescreen size. Smaller viewports scale or scroll within panes
  without hiding essential controls.

## Verification

### Deterministic checks

- Compare the deck's ordered example keys with the tutorial's active includes;
  assert exactly nine example slides and exclude commented-out examples.
- Verify source inclusion reads the canonical sheets and fails explicitly for
  missing source or malformed references.
- Verify host selector validation, asset URL resolution beneath nested paths,
  and explicit loading failures.
- Verify required static assets appear in the output and no generated deck
  content depends on the published book or a runtime CDN.
- Run the existing tutorial tests to preserve the examples' behavior.
- Test any changed Rust staging helper and retain the book staging contract.

### Browser checks

Serve the generated static files under a nested path that resembles GitHub
Pages. Use real-browser automation to inspect screenshots, DOM, and console
errors. Check every slide for mounted controls and legible source.

Exercise representative contracts:

- Edit a first-sheet value and return to the slide to verify persistence.
- Write an out-of-range filter value and observe clamping.
- Change an input and observe its computed output.
- Edit both sides of a relationship and observe graph direction changes.
- Exercise the inequality example's preserved prior values.
- Toggle conditional activation and forced-value editability/restoration.
- Trigger filter and requirement diagnostics.
- Use keyboard and pointer input inside frames without changing slides, then
  navigate through the presentation controls.
- Visit graph slides after other slides and resize the viewport to verify graph
  initialization and layout.

Report missing verification infrastructure explicitly; successful HTML generation
alone does not establish that live slides work.

## Non-goals

- Source editing or an Adam code playground.
- New Adam runtime semantics, inspector controls, or graph features.
- Additional book chapters or commented-out tutorial examples.
- A presentation backend, account system, or network service.
- Live behavior in PDF or PowerPoint exports; interactivity belongs to HTML.
- Unrelated changes to the book's UI or deployment.
