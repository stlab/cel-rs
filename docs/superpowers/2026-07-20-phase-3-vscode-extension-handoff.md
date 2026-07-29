# pm-lang Language Server — Phase 3 Complete

Written after the VS Code extension sub-plan landed, closing out Phase 3 of
`docs/superpowers/specs/2026-07-17-pm-lang-language-server-design.md`. See
`docs/superpowers/2026-07-18-phase-3-handoff.md` for the fuller history of how Phase 3 was split
into three sub-plans (type checking v1, `pm-lsp`, this VS Code extension) and what each one added.

## What's done

All three Phase 3 sub-plans are complete:

1. Type checking v1 (PR #45) — `cel_parser::Ty`/`check_expr`, `pm_lang::typecheck::check_sheet`.
2. `pm-lsp` (PR #46) — the language server binary, `textDocument/publishDiagnostics` over stdio.
3. **VS Code extension (`editors/vscode-pm-lang/`) — this sub-plan.** TextMate grammar for
   `.adm2` syntax highlighting, `vscode-languageclient` wiring to `pm-lsp` over stdio, a
   `pm-lang.serverPath` setting (falling back to the workspace's `target/debug`/`target/release`,
   then `PATH`). First end-to-end usable milestone per the design doc's phasing: open a `.adm2`
   file in VS Code, get highlighting and live diagnostics. The plan and this handoff's earlier
   draft used `.pm` as the file extension; it was changed to `.adm2` after implementation because
   VS Code already strongly associates `.pm` with Perl modules — the demo asset
   (`begin/assets/demo.pm`) was renamed to `begin/assets/demo.adm2` to match, along with every
   hardcoded reference in `begin`'s hot-reload code (`begin/src/demo_source.rs`,
   `begin/src/bridge.rs`, `begin/src/app.rs`).

Full plan: `docs/superpowers/plans/2026-07-20-pm-lang-vscode-extension.md`.

## Outstanding before merge

The extension code and its unit tests are complete and committed, and `pm-lsp` builds cleanly
(`cargo build -p pm-lsp` exits 0). Manual verification (Task 5 Steps 2-3 in
`docs/superpowers/plans/2026-07-20-pm-lang-vscode-extension.md`) is in progress and has already
caught and fixed two dev-host issues along the way: `launch.json`'s auto-open-folder arg breaking
`--extensionDevelopmentPath` recognition, and `File > Open Folder` inside a running dev host
dropping the extension entirely (see the README's "Trying it out" section for the resulting
workflow). Confirm the full checklist — syntax highlighting, live diagnostics end-to-end, and the
`pm-lang.serverPath` override behavior — passes before merging the branch.

## What's left (design doc's later phases, not started)

1. **Formatter** (design doc phase 4) — not started. Would add a `pm-lang` formatting module
   walking the trivia-annotated AST, wired into `pm-lsp`'s `textDocument/formatting` and the
   extension's format-on-save.
2. **Hover, go-to-definition/find-references, completion, document symbols** (design doc phase 5)
   — not started. Built on the same `AstContext`-produced AST; identifier resolution reuses the
   cell-name scoping already established by `pm_lang::parser`'s `cell_names` map concept.

## Still deliberately deferred (unchanged from the 2026-07-18 handoff)

1. `PmParser`'s grammar is still hand-duplicated against `PmAstParser`'s — the required immediate
   follow-on cleanup plan described in the 2026-07-18 handoff has not been started.
2. The known `AstContext` recovery limitation with dangling braces in CEL `if`-expressions
   ([github.com/stlab/cel-rs#43](https://github.com/stlab/cel-rs/issues/43)) is unchanged.
3. A declared-type manifest or in-source syntax for richer static checking of custom types is
   still out of scope, per the design doc.

## Key files this sub-plan added

- `editors/vscode-pm-lang/package.json` — extension manifest, `pm-lang.serverPath` setting,
  `pm-lang` language/grammar contribution.
- `editors/vscode-pm-lang/syntaxes/pm-lang.tmLanguage.json` — the TextMate grammar.
- `editors/vscode-pm-lang/src/serverPath.ts` — `resolveServerPath`, unit-tested in
  `test/serverPath.test.ts`.
- `editors/vscode-pm-lang/src/extension.ts` — `activate`/`deactivate`, the `vscode-languageclient`
  wiring.
