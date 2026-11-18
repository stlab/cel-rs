---
title: "Checked geometry inverse distinguishes exact zero from coefficient underflow"
entity_type: discovery
confidence: 1.00
created: 2026-11-17T21:15:52Z
last_accessed: 2026-11-17T21:15:52Z
last_reinforced: 2026-11-17T21:15:52Z
access_count: 1
sources:
  - "cel-euclid/src/transforms.rs:83-120"
  - "cel-euclid/tests/transforms.rs:170-198"
  - "cel-euclid/tests/transforms.rs:211-235"
tags: ["cel-euclid", "geometry", "inverse", "floating-point"]
tier: knowledge
---

# Checked geometry inverse distinguishes exact zero from coefficient underflow

`cel-euclid`'s checked `inverse` accepts a zero inverse coefficient only when its cofactor is exactly zero; it rejects a rounded-to-zero coefficient with a nonzero exact cofactor, including translation terms. The translation checks compare normalized exact products rather than rounded floating-point products; nonzero subnormals remain valid. Preserve this distinction in ports and changes to inversion. See `cel-euclid/src/transforms.rs:83-120` and `cel-euclid/tests/transforms.rs:170-198,211-235`.
