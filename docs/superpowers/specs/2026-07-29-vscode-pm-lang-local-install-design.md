# pm-lang VS Code Extension — Local Install for Development — Design

## Goal

`editors/vscode-pm-lang/README.md` currently only documents *trying* the extension via the
Extension Development Host (F5) — a throwaway session that closes when the debug session stops.
This project adds a way to install a real, working copy of `pm-lsp` and the `pm-lang` extension
into a developer's regular VS Code, so `.adm2` files get highlighting/diagnostics in normal
day-to-day editing of any repo, not just inside a dev-host window with `demo.adm2` open.

This is packaging/tooling work, not a language or runtime change. No `cel-runtime`,
`cel-parser`, or `pm-lang` crate code is touched.

## `pm-lsp` install

`cargo install --path pm-lsp`, run from the repo root, builds `pm-lsp` and copies the binary to
`~/.cargo/bin`, which is normally on `PATH`. `resolveServerPath` (`src/serverPath.ts`) already
falls back to searching `PATH` as its last resolution step, so once the binary is installed there
the extension finds it with **no `pm-lang.serverPath` setting required**, regardless of which
folder/workspace is open in VS Code — unlike the existing `target/debug`/`target/release`
resolution step, which only fires when the open workspace folder *is* the `cel-rs` checkout.

Re-running `cargo install --path pm-lsp` after a `pm-lsp`/`pm-lang`/`cel-parser` code change
overwrites the installed binary (cargo's default `install` behavior); no extra flag is needed.

## Extension packaging

Add `@vscode/vsce` (`^3.9.2`) as a devDependency of `editors/vscode-pm-lang`, plus:

```json
"scripts": {
  "vscode:prepublish": "npm run compile",
  "package": "vsce package --out pm-lang.vsix",
  "install-extension": "code --install-extension pm-lang.vsix --force"
}
```

- `vsce package`'s `--out pm-lang.vsix` pins one fixed output filename. Without it, vsce names the
  file `pm-lang-<version>.vsix`, which would force `install-extension` to glob for the current
  version — the same cross-shell glob-expansion hazard already flagged (and fixed) on this
  project's `npm test` script, where `cmd.exe` doesn't expand `*` the way POSIX shells do. Pinning
  the name sidesteps that class of bug entirely.
- `vscode:prepublish` is vsce's standard pre-package hook; `vsce package` invokes it automatically,
  so the vsix always ships a freshly-compiled `out/`, even if the developer forgot to run
  `npm run compile` first.
- `--force` on `code --install-extension` allows reinstalling over an already-installed copy at
  the same version (VS Code does not treat a same-version vsix install as a no-op otherwise
  requiring a version bump).

A new `editors/vscode-pm-lang/.vscodeignore` excludes `src/**`, `test/**`, `out/test/**`,
`.vscode/**`, `tsconfig.json`, `**/*.map`, and `package-lock.json`, so the packaged vsix contains
only the compiled runtime (`out/src/**`), the manifest, and the syntax/grammar/configuration
assets — not TypeScript sources, tests, or dev-only config.

`editors/vscode-pm-lang/*.vsix` is added to the repo-root `.gitignore` (alongside the existing
`node_modules`/`out` entries for this extension) — it's a build artifact, not a checked-in file.

## VS Code task

`editors/vscode-pm-lang/.vscode/tasks.json` gains three new tasks plus one combined task:

1. `cargo: install pm-lsp` (`type: shell`, runs `cargo install --path pm-lsp` with
   `options.cwd` set to the repo root, i.e. `${workspaceFolder}/../..`, since this folder's
   `${workspaceFolder}` is `editors/vscode-pm-lang` under this project's existing F5 workflow,
   where VS Code has *this* folder open on its own, not the repo root).
2. `npm: package` (`type: npm`, runs the `package` script above).
3. `npm: install-extension` (`type: npm`, runs the `install-extension` script above).
4. `Install pm-lsp + Extension (Local)` — no command of its own; `dependsOn` the three tasks
   above with `dependsOrder: sequence`, so running this one task from the Command Palette
   ("Tasks: Run Task") does the full install in order.

The existing `compile` build task is unchanged.

## README

A new "Local Install (for development)" section is added after "Trying it out", documenting:

- The two manual commands (`cargo install --path pm-lsp`; `npm run package && npm run
  install-extension`) as the source of truth for what actually happens.
- A one-line pointer that the combined VS Code task does the same thing in one step.
- That re-running both steps is how you pick up a code change (extension or `pm-lsp`) — this
  is a real install, not a live dev loop like F5.
- A note that `code` must be on `PATH` for `install-extension` to work (VS Code's Command Palette
  → "Shell Command: Install 'code' command in PATH" if it isn't yet) — documented as a prerequisite
  rather than checked in code, since a missing `code` command already fails with a clear
  "command not found" from the shell.

## Testing / verification plan

This is packaging tooling, not application logic, so verification is by running the real
commands rather than unit tests:

1. `cargo install --path pm-lsp` from repo root; confirm `pm-lsp`/`pm-lsp.exe` lands in
   `~/.cargo/bin` and is resolvable via `which pm-lsp` / `where pm-lsp.exe`.
2. `npm run package` in `editors/vscode-pm-lang`; confirm `pm-lang.vsix` is produced and, by
   inspecting its contents (`vsce ls` or unzip), that it excludes `src/`, `test/`, and
   `.vscode/` and includes `out/src/**`.
3. `npm run install-extension`; confirm the extension shows installed in VS Code
   (`code --list-extensions`) and, opening any `.adm2` file in a normal (non-dev-host) VS Code
   window with no workspace-relative `target/` build present, that highlighting and live
   diagnostics both work — proving the PATH-based resolution actually fires.
4. Run the combined "Install pm-lsp + Extension (Local)" task from the Command Palette and confirm
   it performs all of the above in one step, in order.
5. `npm test` (existing `serverPath.test.ts` suite) still passes unmodified — this project adds no
   new resolution logic to `serverPath.ts`, only build/packaging tooling around it.
