---
name: verifying-begin-ui
description: Use when validating any UI change to the `begin` Dioxus app - checking rendering, Spectrum styling/theming, layout, DOM structure, or that a component actually upgraded - since `begin` normally runs as a desktop WebView2 window that standard headless-browser tooling (Playwright, chromium-cli) can't attach to, and this environment has no node/Playwright installed.
---

# Verifying begin's UI

## Overview

`begin` defaults to a desktop WebView2 window (`cargo run -p begin`), which no
headless driver here can screenshot or introspect. Serve it as a plain web app
instead, then drive it with the system's Edge browser - no new dependencies
needed. This is how a real, reproducible bug (Spectrum 2 theming completely
missing app-wide) was found and confirmed during the 2026-07-11 session: a
screenshot alone showed "something's unstyled"; a live DevTools query proved
*why* (`sp-theme`'s shadow root had zero adopted stylesheets).

## Procedure

1. **Serve as a web app** (from `begin/`, background it):
   ```bash
   dx serve --platform web --no-default-features --features web --open false > /tmp/dx.log 2>&1 &
   disown
   ```
   `Dioxus.toml` sets `default_platform = "desktop"`, so the platform flags are
   required - without them you get a desktop devtools server that returns
   `Err 404 - dioxus is not currently serving a web app` on port 8080.
   Poll instead of guessing a sleep: `until curl -sf http://localhost:8080 >/dev/null; do sleep 1; done`.

2. **Screenshot** (visual check):
   ```bash
   "/c/Program Files (x86)/Microsoft/Edge/Application/msedge.exe" \
     --headless=new --disable-gpu --no-sandbox --user-data-dir=/tmp/begin-ui-profile \
     --window-size=1280,900 --virtual-time-budget=8000 \
     --screenshot=/tmp/shot.png http://localhost:8080
   ```
   Read `/tmp/shot.png` with the Read tool and actually look at it.

3. **DOM dump** (structural check - catches corrupted tag names, missing
   attributes, wrong element nesting):
   ```bash
   msedge.exe --headless=new --disable-gpu --no-sandbox --user-data-dir=/tmp/begin-ui-profile \
     --virtual-time-budget=8000 --dump-dom http://localhost:8080 > /tmp/dom.html
   ```
   `grep` it for the elements/attributes your change touched.

4. **Live JS/CSS introspection** (when the DOM and screenshot alone don't
   explain the symptom - computed styles, shadow-root contents, custom-element
   registration, adopted stylesheets):
   ```bash
   msedge.exe --headless=new --disable-gpu --no-sandbox --user-data-dir=/tmp/begin-ui-profile \
     --remote-debugging-port=9222 http://localhost:8080 &
   EDGE_PID=$!
   curl -s http://127.0.0.1:9222/json | grep -B3 'localhost:8080'   # find "id"
   python3 cdp_eval.py <that-id> "JSON.stringify({...whatever you need...})"
   ```
   [cdp_eval.py](cdp_eval.py) is a dependency-free raw-WebSocket DevTools
   Protocol client (no node/Playwright/websocket-client required).

## Gotchas

- **Never `taskkill //IM msedge.exe` (or any kill-by-process-name).** The user
  runs Edge as their regular browser - killing by image name takes down their
  own windows too, not just the instance this skill launched. Always capture
  the PID from the launch command (`$!` right after backgrounding it, or from
  the tool's own process-tracking) and kill that specific PID
  (`taskkill //F //PID $EDGE_PID`). The `--user-data-dir=/tmp/begin-ui-profile`
  flag above also matters beyond isolation: without it, headless Edge launches
  against the user's real default profile (confirmed - a bare launch pulled in
  the user's own installed extensions' background pages), which is both slower
  and another way this skill could touch state that isn't its own.
- **Pass the bare target `id` to `cdp_eval.py`, never `/devtools/page/<id>`.**
  git-bash's MSYS layer silently rewrites any argument starting with `/` into
  a bogus Windows path before a native `.exe` sees it (verified:
  `/devtools/page/X` became `C:/Program Files/Git/devtools/page/X`), which
  hangs the WebSocket handshake with **zero output and no exception** - the
  single most confusing failure mode here. `cdp_eval.py` builds the path
  internally specifically to avoid this.
- Use a fully detached background launch (`nohup ... & disown`, or the
  harness's own background-task support) - a bare trailing `&` inside a
  foreground call can die when that call returns. Capture the PID (`$!`)
  before `disown`ing it, or you lose the ability to clean up precisely.
- `taskkill //F //IM dx.exe` is lower-risk (the user doesn't run `dx` as a
  personal app) but prefer PID-based cleanup for it too when convenient.
