---
title: "Graph Channels Require Exclusive Container Ownership"
entity_type: discovery
confidence: 0.97
created: 2026-10-28T23:42:19Z
last_accessed: 2026-10-28T23:42:19Z
last_reinforced: 2026-10-28T23:42:19Z
access_count: 1
sources:
  - "adam-lang-book-live/src/lib.rs:62-72"
  - "adam-lang-book-live/src/lib.rs:115-123"
  - "adam-web-ui/src/graph/channel.js:13-24"
tags: ["graph", "ownership", "live-book", "bootstrap", "identity"]
tier: knowledge
---

# Graph Channels Require Exclusive Container Ownership

A graph container ID identifies one active channel owner, not an example source. `runGraphChannel` retires the previous owner when another owner opens the same ID (adam-web-ui/src/graph/channel.js:13-24). Each live-book `mount` creates a separate VirtualDom, and its root creates GraphViews for every supplied graph ID (adam-lang-book-live/src/lib.rs:115-123,62-72). Therefore assigning the same graph IDs to two inspector roots makes otherwise independent instances replace each other's channel and drive the same graph. Copying their graph-ID arrays does not establish independent ownership. Validate unambiguous inspector-to-container associations before mounting; one inspector may legitimately drive several distinct containers. Repeated source text is not itself invalid: distinct instances need distinct identities, while intentional replacement must retire the old owner.
