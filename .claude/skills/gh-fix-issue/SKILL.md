---
name: gh-fix-issue
description: Use when asked to fix a GitHub issue in this repo, or to work through the issue linked to the current worktree — determines the issue number from the `worktree-issue-#` branch convention (or an explicit number), verifies the issue's claim against this project's own conventions and current code before touching anything, then either implements a fix and opens a PR or closes the issue with an explanation.
---

# Fixing a GitHub Issue

## Overview

Wraps this repo's issue-to-PR loop: find the issue, don't trust it, fix it or reject it. The
mechanics (`gh issue view`, `gh pr create`) aren't the risk — skipping validation and coding up
an issue that's stale, a duplicate, already fixed, or contradicts CLAUDE.md is. Step 3's
verification gate mirrors `pr-receive`'s Project-Guideline Gate, just applied to an issue instead
of a review comment.

**REQUIRED SUB-SKILLS**, invoked in turn as you reach that phase: `superpowers:systematic-debugging`
(investigate before touching code), `superpowers:test-driven-development` (implement the fix),
`superpowers:verification-before-completion` (before opening the PR).

## Step 1: Determine the Issue Number

1. Given an explicit issue number (skill argument, or stated in the request) — use it, skip to Step 2.
2. Otherwise check the current branch:
   ```bash
   git branch --show-current
   ```
   Match against `^worktree-issue-(\d+)$` (this repo's `EnterWorktree` prefixes every worktree
   branch with `worktree-`, so a worktree created for issue 117 sits on branch
   `worktree-issue-117`, even though its directory under `.claude/worktrees/` is just `issue-117`).
3. If neither applies (e.g. on `main`, or a branch that doesn't match), **ask the user** for the
   issue number — do not guess from an unrelated branch name.

## Step 2: Get Into an Isolated Worktree

This repo's CLAUDE.md requires a worktree before any code change. If Step 1 already matched
`worktree-issue-<N>`, you're already in it. Otherwise create one:

```
EnterWorktree(name: "issue-<N>")
```

(this yields directory `issue-<N>` and branch `worktree-issue-<N>`). Fall back to
`superpowers:using-git-worktrees` only if no native worktree tool is available.

## Step 3: Fetch and Evaluate the Issue

```bash
gh issue view <N> --json title,body,state,comments,labels,url
```

If `state` is already `CLOSED`, stop and tell the user rather than redoing or reopening closed work.

**Do not assume the issue is valid.** Verify it the same way `pr-receive` verifies a bot review
comment, before writing any code:

1. **Read the code at every location the issue names** — don't reason from the issue text alone.
2. **Verify the claim independently — don't just re-read the issue's own reasoning.** Where a
   unit test can exercise it, write or run the smallest one that would fail if the bug is real
   (mirrors `pr-receive`'s "Verify by Reproduction, Not by Reasoning Alone"). Where it can't — a
   UI/timing/browser bug, a third-party dependency's behavior — verify some other independent way
   instead (read the actual vendored/library source, drive it live, whatever settles it) rather
   than treating "no test is feasible" as license to accept the issue's reasoning unchecked. For a
   feature request, confirm the described gap still exists on current `main` — issues can be
   filed against code that has since changed.
3. **Check it against this project's own conventions**: CLAUDE.md's Code Style rules (no heap
   allocations where avoidable, fallible ops use `.op1r`/`.op2r`, contract-style doc comments),
   the four-layer stack architecture, "don't add error handling for scenarios that can't happen,"
   etc. A request can be internally coherent and still be wrong for this codebase.
4. **Check for staleness or duplication**: already fixed on `main`? Superseded by a later issue
   or merged PR? Out of scope for one focused change?

| Outcome | Action |
|---|---|
| Real, reproducible, consistent with project conventions | Fix it (Step 4) |
| Already fixed / stale / duplicate | Close with explanation (Step 5), no PR |
| Contradicts a project convention, or is out of scope as filed | Close with explanation (Step 5), no PR — don't silently substitute a different fix than what was asked without checking with the user first |
| Partially valid | Fix the valid part; comment on the issue explaining the part you didn't do, mirroring `pr-receive`'s partial-fix reply, even if you still close it |

## Step 4: Fix It, Then Open a PR

Follow `superpowers:test-driven-development` for the fix. When done, run
`superpowers:verification-before-completion` before claiming it's fixed — an unverified "fixed"
claim is exactly what that skill exists to catch.

Then open the PR with `pr-open`, with `Fixes #<N>` in the PR body so GitHub auto-links and
auto-closes the issue on merge.

## Step 5: Close an Invalid Issue

```bash
gh issue comment <N> --body "<what you checked, and why the issue doesn't hold>"
gh issue close <N>
```

Comment first, close second — same ordering as `pr-receive`'s "reply first, resolve second," so
there's something for a human to read before the issue drops off the open list. Never close
without a comment.

## Common Mistakes

| Mistake | Fix |
|---|---|
| Coding before checking the issue's claim | Reproduce it first — an issue can be filed against stale code |
| Trusting the issue's stated file/line without reading current code | Read the actual code at every named location, every time |
| Closing an issue with no comment | Always comment first — a silent close loses the reasoning |
| Guessing the issue number from a worktree/branch that doesn't match the convention | Ask the user instead of guessing |
| Opening a PR before running verification-before-completion | Run it first — don't open a PR on an unverified "fixed" claim |
