---
title: "Affine vector mapping must ignore nonfinite translation"
entity_type: pattern
confidence: 0.99
created: 2026-11-17T21:40:08Z
last_accessed: 2026-11-17T21:40:08Z
last_reinforced: 2026-11-17T21:40:08Z
access_count: 1
sources:
  - "cel-lang-book/book-src/geometry.md:1633-1646"
  - "cel-euclid/src/transforms.rs:253-257"
  - "cel-euclid/SIGNATURES.md:69-75"
tags: ["geometry", "affine-transform", "floating-point", "portability"]
tier: knowledge
---

# Affine vector mapping must ignore nonfinite translation

`transform_vector` applies only the 2×2 linear coefficients; `m31` and `m32` never contribute, even when they are nonfinite. Do not implement it by multiplying a full homogeneous matrix by `(x, y, 0)`: `0 * infinity` can introduce a NaN absent from the CEL contract. Use the linear matrix or the two documented component expressions.
