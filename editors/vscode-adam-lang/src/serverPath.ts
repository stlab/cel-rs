import * as path from 'node:path';

/** Inputs needed to resolve the `adam-lsp` binary's filesystem location. */
export interface ResolveServerPathOptions {
  /** The user's `adam-lang.serverPath` setting, if set. */
  configuredPath: string | undefined;
  /** The first workspace folder's filesystem path, if any workspace is open. */
  workspaceRoot: string | undefined;
  /** `process.platform`, injected so tests can exercise Windows and Unix naming. */
  platform: NodeJS.Platform;
  /** `process.env.PATH`, injected so tests don't depend on the real environment. */
  pathEnv: string | undefined;
  /** Checks whether a file exists at the given path, injected so tests avoid real disk I/O. */
  fileExists: (candidate: string) => boolean;
}

const ADAM_LSP_UNIX = 'adam-lsp';
const ADAM_LSP_WINDOWS = 'adam-lsp.exe';

/** Returns the `adam-lsp` binary name for `platform` (`adam-lsp.exe` on Windows, `adam-lsp` elsewhere). */
function binaryName(platform: NodeJS.Platform): string {
  return platform === 'win32' ? ADAM_LSP_WINDOWS : ADAM_LSP_UNIX;
}

/**
 * Resolves the filesystem path of the `adam-lsp` binary to launch.
 *
 * Resolution order:
 * 1. `options.configuredPath`, trimmed, if non-empty — used only if it exists; a configured path
 *    that doesn't exist is a user error and must not silently fall through to auto-detection. A
 *    whitespace-only setting is treated as unset and falls through to step 2.
 * 2. `<workspaceRoot>/target/debug/<binary>`, then `<workspaceRoot>/target/release/<binary>`.
 * 3. Each directory in `options.pathEnv` (in order), joined with `<binary>`.
 *
 * Returns `undefined` if none of the above exist.
 */
export function resolveServerPath(options: ResolveServerPathOptions): string | undefined {
  const { configuredPath, workspaceRoot, platform, pathEnv, fileExists } = options;
  const binary = binaryName(platform);
  const pathModule = platform === 'win32' ? path.win32 : path.posix;

  const trimmedConfiguredPath = configuredPath?.trim();
  if (trimmedConfiguredPath) {
    return fileExists(trimmedConfiguredPath) ? trimmedConfiguredPath : undefined;
  }

  if (workspaceRoot) {
    for (const profile of ['debug', 'release']) {
      const candidate = pathModule.join(workspaceRoot, 'target', profile, binary);
      if (fileExists(candidate)) {
        return candidate;
      }
    }
  }

  if (pathEnv) {
    const delimiter = platform === 'win32' ? ';' : ':';
    for (const rawDir of pathEnv.split(delimiter)) {
      const dir = rawDir.trim();
      if (!dir) {
        continue;
      }
      const candidate = pathModule.join(dir, binary);
      if (fileExists(candidate)) {
        return candidate;
      }
    }
  }

  return undefined;
}
