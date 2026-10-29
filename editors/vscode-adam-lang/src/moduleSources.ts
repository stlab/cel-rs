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
  return new ModuleSourceLoader(() => options).load();
}

/** Owns a lexical mapping and its last approved canonical dependency, even after IO failure. */
interface ModuleFileOwner {
  readonly file: ModuleFile;
  readonly workspaceRoot: string;
  target?: string;
  root?: string;
  unresolved: boolean;
}

/**
 * Owns canonical event aliases separately from lexical wire/overlay identities.
 * Failed realpath resolution temporarily selects all source events for recovery;
 * successful resolution restores precise dependency selection, even if reading fails.
 * Configuration changes prune removed owners; successful retargeting replaces aliases.
 * Load calls must not overlap; event selection may run during a suspended load.
 * Retained space: O(p) configured/canonical path bytes, excluding source snapshots.
 * Example: const sources = new ModuleSourceLoader(currentOptions); await sources.load().
 */
export class ModuleSourceLoader {
  private owners: ReadonlyMap<string, ModuleFileOwner> = new Map();

  /** Retains the caller-owned current-options provider without capturing a configuration snapshot. */
  constructor(private readonly currentOptions: () => ModuleSourceOptions) {}

  /**
   * Loads lexical wire records after validating canonical containment for every owner.
   * Resolves all owners even when another owner fails, retaining aliases before reads.
   * Uses one options/owner snapshot throughout suspension; detached owners cannot update
   * replacement configurations' aliases or failure state.
   * Rejects invalid configuration, physical escapes and IO failures without partial success.
   * Time/space: O(p + t) plus IO for path bytes p and source bytes t.
   * Example: const records = await sources.load().
   */
  async load(): Promise<readonly ModuleSource[]> {
    const options = this.currentOptions();
    this.synchronize(options);
    const owners = this.owners;
    if (owners.size === 0) { return []; }
    let root: string;
    try {
      root = await options.realPath(options.workspaceRoot!);
    } catch (error: unknown) {
      // The shared root must resolve before any owner's canonical dependency is known.
      for (const owner of owners.values()) { owner.unresolved = true; }
      throw error;
    }
    const records: ModuleSource[] = [];
    let failure: Error | undefined;
    // Visit every owner so a failed module cannot hide later modules' event dependencies.
    for (const owner of owners.values()) {
      const { modulePath, file, absolute, uri } = owner.file;
      let target: string;
      try {
        target = await options.realPath(absolute);
        if (!isContained(root, target)) {
          throw new Error('module target must stay inside the workspace');
        }
        owner.target = target;
        owner.root = root;
        owner.unresolved = false;
      } catch (error: unknown) {
        owner.unresolved = true;
        failure ??= new Error(`Cannot read module "${modulePath}" (${file}): ${errorMessage(error)}`);
        continue;
      }
      try {
        records.push({ path: modulePath, uri, text: await options.readText(target) });
      } catch (error: unknown) {
        failure ??= new Error(`Cannot read module "${modulePath}" (${file}): ${errorMessage(error)}`);
      }
    }
    if (failure) { throw failure; }
    return records;
  }

  /**
   * Selects lexical/canonical files and their ancestor replacement events, including
   * canonical targets rebased beneath the lexical workspace root used by watchers.
   * Unresolved realpaths select every event until a subsequent load resolves them.
   * Invalid configuration selects no events; loading reports that error.
   * Time: O(p * d) for path bytes p and maximum directory depth d; space: O(p).
   * Example: sources.isSource("file:///repo/physical/library.txt").
   */
  isSource(uri: string): boolean {
    const options = this.currentOptions();
    try {
      this.synchronize(options);
    } catch {
      return false;
    }
    // Inspect owner dependencies without materializing another collection.
    for (const owner of this.owners.values()) {
      if (owner.unresolved ||
          isDependencyUri(uri, owner.file.absolute, owner.workspaceRoot, options.toUri) ||
          (owner.target !== undefined && owner.root !== undefined &&
            (isDependencyUri(uri, owner.target, owner.root, options.toUri) ||
              isDependencyUri(uri, path.resolve(owner.workspaceRoot, path.relative(owner.root, owner.target)),
                owner.workspaceRoot, options.toUri)))) {
        return true;
      }
    }
    return false;
  }

  /**
   * Rebuilds current lexical ownership while preserving only still-configured aliases.
   * Invalid configuration clears aliases and propagates the validation error.
   * Time/space: O(p) configured path bytes.
   */
  private synchronize(options: ModuleSourceOptions): void {
    try {
      const files = resolveModuleFiles(options);
      const workspaceRoot = options.workspaceRoot === undefined ? '' : path.resolve(options.workspaceRoot);
      this.owners = new Map(files.map((file) => {
        const previous = this.owners.get(file.uri);
        // Reuse only the same generation; suspended loads retain any detached owners.
        const owner = previous?.file.modulePath === file.modulePath &&
          previous.file.absolute === file.absolute && previous.file.file === file.file &&
          previous.workspaceRoot === workspaceRoot
          ? previous : { file, workspaceRoot, unresolved: false };
        return [file.uri, owner];
      }));
    } catch (error: unknown) {
      this.owners = new Map();
      throw error;
    }
  }
}

/**
 * Selects an exact file or an ancestor whose replacement can change that dependency.
 * Precondition: absolute and root are absolute paths, and absolute is contained in root.
 * Time: O(p * d) for path bytes p and directory depth d; space: O(p).
 */
function isDependencyUri(uri: string, absolute: string, root: string, toUri: (file: string) => string): boolean {
  const boundary = path.resolve(root);
  for (let file = absolute; ; file = path.dirname(file)) {
    if (toUri(file) === uri) { return true; }
    if (file === boundary || file === path.dirname(file)) { return false; }
  }
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
  /** Schedules a refresh for a current source dependency or unresolved-path recovery. */
  readonly sourceChanged: (uri: string) => void;
  /** Schedules a refresh only when the modules configuration changed. */
  readonly configurationChanged: (affectsModules: boolean) => void;
}

/**
 * Creates editor-event handlers that select refreshes from the current configuration.
 * The source owner supplies only identity checks; the scheduler remains caller-owned.
 * Construction takes O(1) time and space. Each event adds O(1) handler overhead
 * to the supplied operations' time and space: source events call isSource, and
 * selected source/configuration events call requestRefresh. URI conversion before
 * dispatch belongs to the caller; conversion within isSource belongs to its cost.
 * Example: createModuleSourceHandlers(sources, requestRefresh).configurationChanged(true).
 */
export function createModuleSourceHandlers(
  sources: Pick<ModuleSourceLoader, 'isSource'>,
  requestRefresh: () => void,
): ModuleSourceHandlers {
  return {
    sourceChanged: (uri) => {
      if (sources.isSource(uri)) { requestRefresh(); }
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
