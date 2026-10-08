---
title: "Clippy and Feature Flag Validation Matrix"
entity_type: procedure
confidence: 0.85
created: 2026-10-08T18:52:22Z
last_accessed: 2026-10-08T18:52:22Z
last_reinforced: 2026-10-08T18:52:22Z
access_count: 1
sources:
  - "Cargo.toml"
relationships:
  - type: applies_to
    target: "Architecture Overview"
tags: ["build", "clippy", "feature-flags", "validation", "cargo"]
tier: knowledge
source: inferred
---

# Clippy and Feature Flag Validation Matrix

Workspace CI and local linting require five separate clippy invocations to cover desktop-only and platform-independent targets without spurious warnings:
1. `cargo clippy --workspace --exclude begin --all-targets -- -D warnings`: Core libraries, CLI tools, tests, and benchmarks.
2. `cargo clippy -p begin --no-default-features --all-targets -- -D warnings`: begin headless / non-desktop build.
3. `cargo clippy -p begin --all-targets -- -D warnings`: begin desktop build with platform renderer.
4. `cargo clippy -p ez-adam --no-default-features --all-targets -- -D warnings`: ez-adam non-desktop build.
5. `cargo clippy -p ez-adam --all-targets -- -D warnings`: ez-adam desktop build.
Documentation validation additionally requires: `RUSTDOCFLAGS="-D warnings" cargo doc --lib --no-deps --workspace`.

Aliases / also known as: clippy matrix, cargo clippy flags, feature flags, begin no-default-features, ez-adam features, build validation
