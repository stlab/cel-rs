---
title: "Begin desktop recovery tests share the examples watcher"
entity_type: discovery
confidence: 0.98
created: 2026-11-18T03:14:51Z
last_accessed: 2026-11-18T03:14:51Z
last_reinforced: 2026-11-18T03:14:51Z
access_count: 1
sources:
  - "begin/src/app_tests.rs:269-294"
  - "begin/src/app_tests.rs:323-385"
  - "begin/src/app.rs:285-323"
  - "begin/src/app.rs:94-103"
  - "begin/build.rs:24-35"
  - "begin/src/recovery.rs:193-216"
tags: ["begin", "desktop", "tests", "watcher", "recovery"]
tier: knowledge
---

# Begin desktop recovery tests share the examples watcher

Begin desktop recovery tests mount real watchers on the shared `begin/examples` directory. One recovery fixture retains the source name `cycle` while another test creates and rewrites an example file; the directory event can reload the real cyclic example and terminate the controlling test process. An interrupted `__test_recovery_<pid>.adm2` fixture is then embedded by `begin/build.rs`, causing later authored-example tests to fail. Until #293 is fixed, run the workspace suite with `RUST_TEST_THREADS=1` and set `BEGIN_TEST_RECOVERY_PATH` to a session-local path; remove only test fixtures known to belong to the interrupted run.
