import * as path from 'node:path';

/** Inputs needed to resolve the `pm-lsp` binary's filesystem location. */
export interface ResolveServerPathOptions {
  /** The user's `pm-lang.serverPath` setting, if set. */
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

const PM_LSP_UNIX = 'pm-lsp';
const PM_LSP_WINDOWS = 'pm-lsp.exe';

/** Returns the `pm-lsp` binary name for `platform` (`pm-lsp.exe` on Windows, `pm-lsp` elsewhere). */
function binaryName(platform: NodeJS.Platform): string {
  return platform === 'win32' ? PM_LSP_WINDOWS : PM_LSP_UNIX;
}

/**
 * Resolves the filesystem path of the `pm-lsp` binary to launch.
 *
 * Resolution order:
 * 1. `options.configuredPath`, if non-empty — used only if it exists; a configured path that
 *    doesn't exist is a user error and must not silently fall through to auto-detection.
 * 2. `<workspaceRoot>/target/debug/<binary>`, then `<workspaceRoot>/target/release/<binary>`.
 * 3. Each directory in `options.pathEnv` (in order), joined with `<binary>`.
 *
 * Returns `undefined` if none of the above exist.
 */
export function resolveServerPath(options: ResolveServerPathOptions): string | undefined {
  const { configuredPath, workspaceRoot, platform, pathEnv, fileExists } = options;
  const binary = binaryName(platform);
  const pathModule = platform === 'win32' ? path.win32 : path.posix;

  if (configuredPath) {
    return fileExists(configuredPath) ? configuredPath : undefined;
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
    for (const dir of pathEnv.split(delimiter)) {
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
