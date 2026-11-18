---
title: "Outer geometry bounds preserve first equal extrema"
entity_type: discovery
confidence: 0.70
created: 2026-11-17T23:09:33Z
last_accessed: 2026-11-17T23:09:33Z
last_reinforced: 2026-11-17T23:09:33Z
access_count: 1
sources:
  - "cel-euclid/src/transforms.rs:198-216"
  - "euclid-0.22.14/src/box2d.rs:457-489"
tags: ["cel-euclid", "geometry", "floating-point"]
tier: knowledge
relationships:
  - type: relates_to
    target: "Transformed outer bounds must enclose corners without claiming interior coverage"
  - type: relates_to
    target: "Checked geometry inverse distinguishes exact zero from coefficient underflow"
---

# Outer geometry bounds preserve first equal extrema

`outer_transformed_rect` delegates corner extrema to Euclid's `Box2D::from_points`, which keeps the first equal minimum and maximum. Signed zeros make this observable in the result's extent: `std::minmax_element` chooses the last equal maximum and can return `-0.0` where Euclid returns `+0.0`. Preserve first-tie extrema when porting this operation.
