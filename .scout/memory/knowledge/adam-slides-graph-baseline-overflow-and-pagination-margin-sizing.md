---
title: "Adam slides graph baseline overflow and pagination margin sizing"
entity_type: discovery
confidence: 0.99
created: 2026-10-09T03:31:36Z
last_accessed: 2026-10-09T04:52:22Z
last_reinforced: 2026-10-09T04:52:22Z
access_count: 2
sources:
  - "adam-slides/example.css:33-35"
  - "adam-slides/slides.css:4-14"
  - "adam-slides/slides.css:27-32"
  - "adam-slides/slides.css:54-62"
  - "adam-slides/tests/slides.spec.mjs:48-99"
  - "adam-slides/example.css:18-40"
  - "adam-slides/slides.css:26-32"
tags: ["adam-slides", "marp", "css", "svg", "layout", "scrollbars"]
tier: knowledge
---

# Adam slides graph baseline overflow and pagination margin sizing

Adam slides reuse the existing graph driver inside static same-site frames. A graph SVG with default inline display can introduce a text-baseline gap even when its graph container and inspector exactly fit the frame. In Chromium, the Out cells host measured clientHeight 524 and scrollHeight 528; changing only the SVG to display:block removed the four-pixel overflow. Preserve the slide-local `#graph svg { display: block; }` rule at adam-slides/example.css:38-40; do not hide overflow or shrink controls to mask this symptom.

Outer sizing has two related constraints: iframe borders must be included in the allotted 100% width/height via box-sizing:border-box, and Marp's section::after pagination must fit wholly inside the reserved bottom margin. The section reserves 42px below its panes; pagination uses bottom:10px, font-size:18px and line-height:1. Moving pagination downward without constraining its inherited typography was insufficient: its inherited 24px font/36px line box still overlapped the pane.

A separate later cause of overflow was a fixed 240px graph minimum combined with extra slide prose: the inspector plus graph minimum exceeded shortened frame height. Graph-bearing main now uses height:100vh/min-height:0, the inspector does not shrink, and the graph uses flex:1/min-height:0 (adam-slides/example.css:18-35). The slide section uses a flexible column rather than assuming exactly three grid rows.

Regression checks at adam-slides/tests/slides.spec.mjs:48-99 wait for visible graph SVGs and fonts, compare document scroll dimensions with frame dimensions, and verify pagination stays below panes at 1280x720 and 960x540. The acceptance deck is independent of authoring content and includes extra prose. All 27 Chromium checks passed after the flexible-authoring follow-up; screenshots of both a local example and Out cells with additional prose were inspected.
