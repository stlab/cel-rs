---
title: "Keep Begin test fixtures out of examples/"
entity_type: pattern
confidence: 0.80
created: 2026-11-18T06:57:01Z
last_accessed: 2026-11-18T06:57:01Z
last_reinforced: 2026-11-18T06:57:01Z
access_count: 1
tags: []
tier: knowledge
---

# Keep Begin test fixtures out of examples/

Begin tests that exercise mounted-app reloads should keep temporary source files outside `begin/examples/` and use `SourceOrigin::Opened` when testing opened-file reloads. `spawn_examples_watch` observes additions, removals, and edits in that directory, and `AppWithExample` handles each event by reloading the active source (`begin/src/example_source.rs:250-257`, `begin/src/app.rs:290-312`). The build script scans every `.adm2` there and emits each source through `include_str!` for tests and non-desktop builds (`begin/build.rs:25-38`, `begin/build.rs:46-60`). In-tree fixtures can therefore cross-talk with mounted apps and pollute compile-time test inputs; use a temporary directory instead.
