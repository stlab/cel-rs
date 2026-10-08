---
title: "Git Worktree and No-Direct-Commit Policy"
entity_type: procedure
confidence: 0.85
created: 2026-10-08T18:52:22Z
last_accessed: 2026-10-08T18:52:22Z
last_reinforced: 2026-10-08T18:52:22Z
access_count: 1
sources:
  - ".githooks/pre-commit"
relationships:
  - type: applies_to
    target: "Architecture Overview"
tags: ["git", "worktree", "githooks", "branching", "repository-policy"]
tier: knowledge
source: inferred
---

# Git Worktree and No-Direct-Commit Policy

The repository enforces strict git workflow rules to protect main and avoid uncommitted drift:
- No direct commit: Committing directly to main is prohibited.
- Worktree requirement: All edits, additions, and deletions must occur in a dedicated git worktree under `.claude/worktrees/`.
- Pre-commit hooks: Activate shared git hooks after clone via `git config core.hooksPath .githooks`. The pre-commit hook rejects commits on main or master and checks `cargo fmt --all --check`; run `cargo fmt --all` before committing to apply formatting.
- Unstable API status: cel-rs is unreleased; prefer clean redesigns over compatibility layers or incremental workarounds.

Aliases / also known as: git worktree policy, no direct commit, githooks, branch workflow, repository conventions
