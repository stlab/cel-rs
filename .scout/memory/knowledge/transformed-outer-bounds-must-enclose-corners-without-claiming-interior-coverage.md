---
title: "Transformed outer bounds must enclose corners without claiming interior coverage"
entity_type: discovery
confidence: 1.00
created: 2026-11-17T21:15:59Z
last_accessed: 2026-11-17T21:15:59Z
last_reinforced: 2026-11-17T21:15:59Z
access_count: 1
sources:
  - "cel-euclid/src/transforms.rs:125-168"
  - "cel-euclid/src/transforms.rs:174-216"
tags: ["cel-euclid", "geometry", "bounds", "floating-point"]
tier: knowledge
relationships:
  - type: relates_to
    target: "Checked geometry inverse distinguishes exact zero from coefficient underflow"
---

# Transformed outer bounds must enclose corners without claiming interior coverage

`outer_transformed_rect` checks representability of source endpoints and projected edges, maps all four corners, and advances an extent by one ULP when reconstructing its maximum would round inward. It permits zero extents and singular transforms but rejects collapsed positive projected spans. The resulting axis-aligned outer bound does not establish containment within the transformed shape; crop coverage must be checked separately. See `cel-euclid/src/transforms.rs:125-168,174-216`.
