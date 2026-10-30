---
title: "Numeric Editor Capability Follows The Complete Type Domain"
entity_type: decision
confidence: 1.00
created: 2026-10-29T02:14:17Z
last_accessed: 2026-10-29T02:14:17Z
last_reinforced: 2026-10-29T02:14:17Z
access_count: 1
sources:
  - "adam-web-ui/src/labels.rs:35-54"
  - "adam-web-ui/src/labels.rs:335-358"
  - "adam-web-ui/src/inspector.rs:714-730"
  - "adam-web-ui/src/inspector.rs:861-885"
tags: ["ui", "numeric", "precision", "type-domain", "spectrum"]
tier: knowledge
---

# Numeric Editor Capability Follows The Complete Type Domain

Select numeric widgets from the complete Rust type domain, not the current value or existence of range metadata. `mark_numeric<T>` sets `number_input_safe` only for floats or integer storage up to four bytes; pointer integers therefore follow the compilation target width. Wider signed/unsigned integers use exact string-valued controls even at zero, with ranges, and for readonly outputs. Spectrum number/slider widgets pass through binary64 and round values beyond its exact integer range. Keep numeric parsing/clamp semantics separate from widget capability: wide numeric text still uses the shared normalized/focused draft bridge, while literal String fields remain literal. Actual Begin and EZ browser tests verified grouped 9007199254740993, signed endpoints, readonly/ranged display, blur and derived writes.
