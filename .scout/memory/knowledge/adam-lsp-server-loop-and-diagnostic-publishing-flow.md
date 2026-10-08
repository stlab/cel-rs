---
title: "Adam-lsp Server Loop and Diagnostic Publishing Flow"
entity_type: observation
confidence: 0.75
created: 2026-10-08T18:52:22Z
last_accessed: 2026-10-08T18:52:22Z
last_reinforced: 2026-10-08T18:52:22Z
access_count: 1
sources:
  - "adam-lsp/src/dispatch.rs:25-75"
  - "adam-lsp/src/diagnostics.rs:1-41"
  - "adam-lsp/src/dispatch.rs:215-231"
relationships:
  - type: applies_to
    target: "Adam-lang Dual Parsing Pipeline"
tags: ["adam-lsp", "lsp", "diagnostics", "formatting", "server-loop"]
tier: knowledge
source: inferred
---

# Adam-lsp Server Loop and Diagnostic Publishing Flow

adam-lsp implements the Language Server Protocol for .adm2 files (adam-lsp/src/dispatch.rs:25-75):
- Handshake & Capabilities: Initializes via Connection::initialize with full document sync (TextDocumentSyncKind::FULL) and document formatting provider.
- Request & Notification Dispatch: In main_loop, handles textDocument/didOpen and textDocument/didChange by updating in-memory document state and triggering diagnostics publishing via diagnostics_for_source (adam-lsp/src/diagnostics.rs:1-41).
- Diagnostic generation: Parses the buffer using AdamAstParser, then calls check_sheet with a TypeRegistry. diagnostics_for_source combines recovered syntax errors and type errors into LSP Diagnostic objects.
- Formatting: Calls adam_lang::format_source in-process and returns one whole-document text edit. Sources with syntax errors receive no formatting edits.

Aliases / also known as: adam-lsp, language server, diagnostics_for_source, lsp dispatch, didChange, adam lsp flow
