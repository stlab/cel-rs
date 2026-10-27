import * as path from 'node:path';

/**
 * Owns one initialization/refresh wire record: an explicit module path, absolute
 * source URI and complete text. All fields are immutable values, not file handles.
 * Example: { path: "constraints", uri: "file:///repo/constraints.adm2", text: "sheet ordered() {}" }.
 */
export interface ModuleSource {
  readonly path: string;
  readonly uri: string;
  readonly text: string;
}

/**
 * Supplies explicit workspace-relative module mappings and fallible tooling IO.
 * Example: { modules: { lib: "lib.adm2" }, workspaceRoot, toUri, realPath, readText }.
 */
export interface ModuleSourceOptions {
  readonly modules: unknown;
  readonly workspaceRoot: string | undefined;
  readonly toUri: (absolutePath: string) => string;
  /** Resolves filesystem links to the canonical absolute path, rejecting IO failures. */
  readonly realPath: (absolutePath: string) => Promise<string>;
  /** Reads the approved canonical path, not the original lexical mapping. */
  readonly readText: (absolutePath: string) => Promise<string>;
}

/** Retains an immutable explicit module identity and its resolved workspace-relative file. */
interface ModuleFile {
  readonly modulePath: string;
  readonly file: string;
  readonly absolute: string;
  readonly uri: string;
}

/**
 * Carries a complete replacement snapshot or a visible failure, never an empty-source substitute.
 * Example: { moduleSourcesError: "Cannot read module lib" }.
 */
export type ModuleSourceSnapshot =
  | { readonly moduleSources: readonly ModuleSource[] }
  | { readonly moduleSourcesError: string };

/**
 * Loads only explicitly configured modules into independently owned wire records.
 * Rejects invalid mappings, ambiguous URIs, lexical/physical escapes and IO failures.
 * Retains lexical URIs for editor overlays; reads only approved canonical targets.
 * Time/space: O(p + t) plus IO for p configured/canonical path bytes and t source bytes.
 * Example: await loadModuleSources({ modules: { lib: "lib.adm2" }, workspaceRoot, toUri, realPath, readText }).
 */
export async function loadModuleSources(options: ModuleSourceOptions): Promise<readonly ModuleSource[]> {
  const records: ModuleSource[] = [];
  const files = resolveModuleFiles(options);
  if (files.length === 0) { return records; }
  // Resolve the root too: the workspace folder itself may be a valid symlink.
  const root = await options.realPath(options.workspaceRoot!);
  for (const { modulePath, file, absolute, uri } of files) {
    try {
      const target = await options.realPath(absolute);
      if (!isContained(root, target)) {
        throw new Error('module target must stay inside the workspace');
      }
      records.push({ path: modulePath, uri, text: await options.readText(target) });
    } catch (error: unknown) {
      throw new Error(`Cannot read module "${modulePath}" (${file}): ${errorMessage(error)}`);
    }
  }
  return records;
}

/**
 * Reports whether an absolute target remains within an absolute root.
 * Time/space: O(p) path bytes.
 */
function isContained(root: string, target: string): boolean {
  const relative = path.relative(root, target);
  return relative !== '..' && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
}

/**
 * Selects filesystem events only for explicitly configured files, without reading them.
 * Invalid configuration returns false; loading/refresh reports that configuration error.
 * Time/space: O(n) configured paths.
 * Example: isConfiguredModuleFile("file:///repo/lib.adm2", options).
 */
export function isConfiguredModuleFile(uri: string, options: ModuleSourceOptions): boolean {
  try {
    return resolveModuleFiles(options).some((file) => file.uri === uri);
  } catch {
    return false;
  }
}

/**
 * Exposes immutable editor-event decisions without depending on VS Code.
 * See createModuleSourceHandlers for construction and an example.
 */
export interface ModuleSourceHandlers {
  /** Schedules a refresh only when the URI belongs to the current explicit mapping. */
  readonly sourceChanged: (uri: string) => void;
  /** Schedules a refresh only when the modules configuration changed. */
  readonly configurationChanged: (affectsModules: boolean) => void;
}

/**
 * Creates editor-event handlers that select refreshes from the current configuration.
 * The options provider and scheduler remain caller-owned; no options snapshot is retained.
 * Source-event time/space: O(n) configured paths; configuration events take O(1).
 * Example: createModuleSourceHandlers(currentOptions, requestRefresh).configurationChanged(true).
 */
export function createModuleSourceHandlers(
  currentOptions: () => ModuleSourceOptions,
  requestRefresh: () => void,
): ModuleSourceHandlers {
  return {
    sourceChanged: (uri) => {
      if (isConfiguredModuleFile(uri, currentOptions())) { requestRefresh(); }
    },
    configurationChanged: (affectsModules) => {
      if (affectsModules) { requestRefresh(); }
    },
  };
}

/**
 * Resolves and validates the explicit mapping without filesystem IO.
 * Rejects malformed configuration and duplicate source ownership.
 * Time/space: O(n) configured path bytes.
 */
function resolveModuleFiles(options: ModuleSourceOptions): readonly ModuleFile[] {
  const { modules, workspaceRoot, toUri } = options;
  if (typeof modules !== 'object' || modules === null || Array.isArray(modules)) {
    throw new Error('adam-lang.modules must map module paths to workspace-relative file paths');
  }
  const files: ModuleFile[] = [];
  const uris = new Set<string>();
  for (const [modulePath, file] of Object.entries(modules)) {
    if (!modulePath.split('::').every((segment) => /^(?!_$)[_\p{ID_Start}][_\p{ID_Continue}]*$/u.test(segment))) {
      throw new Error(`Invalid module path "${modulePath}"`);
    }
    if (!workspaceRoot) { throw new Error(`Module "${modulePath}" requires an open workspace folder`); }
    if (typeof file !== 'string' || !file.trim() || path.isAbsolute(file)) {
      throw new Error(`Module "${modulePath}" requires a workspace-relative file path`);
    }
    const absolute = path.resolve(workspaceRoot, file);
    if (!isContained(workspaceRoot, absolute)) {
      throw new Error(`Module "${modulePath}" must stay inside the workspace`);
    }
    const uri = toUri(absolute);
    if (uris.has(uri)) { throw new Error(`Module "${modulePath}" repeats source URI ${uri}`); }
    uris.add(uri);
    files.push({ modulePath, file, absolute, uri });
  }
  return files;
}

/**
 * Returns the observable text of a caught error without assuming its runtime type.
 * Time/space: O(n) for string conversion of n output characters.
 * Example: errorMessage(new Error("denied")) === "denied".
 */
export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Owns a refresh queue; disposal suppresses queued loads and in-flight completion effects.
 * Example: const refresh = createModuleSourceRefresh(load, publish, report); refresh.dispose().
 */
export interface ModuleSourceRefresh {
  /** Loads/publishes one snapshot in invocation order, unless disposed. */
  (): Promise<void>;
  /** Stops future loads and suppresses unfinished load publications/errors. */
  dispose(): void;
}

/**
 * Creates a serialized refresh operation; each call loads and publishes in invocation order.
 * Load failures publish moduleSourcesError and call reportError before the next refresh.
 * Publication failures reject that invocation but do not prevent later refreshes.
 * Time/space: the load/publication costs and O(q) queued promises for q pending refreshes.
 * Example: const refresh = createModuleSourceRefresh(load, publish, report); await refresh().
 */
export function createModuleSourceRefresh(
  load: () => Promise<readonly ModuleSource[]>,
  publish: (snapshot: ModuleSourceSnapshot) => Promise<void>,
  reportError: (message: string) => void,
): ModuleSourceRefresh {
  let pending = Promise.resolve();
  let disposed = false;
  const refresh: ModuleSourceRefresh = () => {
    const next = pending.then(async () => {
      if (disposed) { return; }
      let snapshot: ModuleSourceSnapshot;
      try {
        snapshot = { moduleSources: await load() };
      } catch (error: unknown) {
        if (disposed) { return; }
        const message = errorMessage(error);
        reportError(message);
        snapshot = { moduleSourcesError: message };
      }
      if (!disposed) { await publish(snapshot); }
    });
    pending = next.catch(() => undefined);
    return next;
  };
  refresh.dispose = () => { disposed = true; };
  return refresh;
}
