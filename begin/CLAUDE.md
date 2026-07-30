# CLAUDE.md

This file provides guidance to Claude Code when working with files in `begin/`.

## UI Verification

`cargo build`/`cargo clippy` passing proves `begin`'s UI code compiles — it proves
nothing about what renders. Before considering any UI change to `begin` complete,
actually render it and look: use the `verifying-begin-ui` skill
(`.claude/skills/verifying-begin-ui/SKILL.md`), which serves `begin` as a web app and
drives headless Edge to screenshot it, dump its DOM, and (when needed) query live
computed styles/shadow-DOM state — `begin`'s default desktop WebView2 window can't be
driven by standard headless-browser tooling. A change that "looks right" in the RSX is
not verified until you've seen it rendered.
