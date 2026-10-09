import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { prepareDeck } from "./build.mjs";
import * as build from "./build.mjs";

/** Catches accepting stale output after a failed staging subprocess. */
test("failed asset staging rejects a build even when old output exists", async () => {
  const root = await mkdtemp(join(tmpdir(), "adam-slides-build-"));
  try {
    const output = join(root, "adam-slides", "dist");
    await mkdir(output, { recursive: true });
    await writeFile(join(output, "index.html"), "stale");
    await assert.rejects(prepareDeck(root), /staging|Cargo.toml/i);
    assert.equal(await readFile(join(output, "index.html"), "utf8"), "stale");
    await assert.rejects(readFile(join(output, "slides.md")), { code: "ENOENT" });
  } finally {
    await rm(root, { recursive: true });
  }
});

/** Catches losing local source bytes or flattening nested file identities. */
test("local examples load exact source bytes under reserved keys", async () => {
  const directory = await mkdtemp(join(tmpdir(), "adam-slides-local-"));
  try {
    await mkdir(join(directory, "nested"));
    await writeFile(join(directory, "custom.adm2"), "sheet custom {}\n");
    await writeFile(join(directory, "nested", "other.adm2"), "sheet other {}");
    await writeFile(join(directory, "README.txt"), "Not an example");
    assert.deepEqual(await build.readLocalExamples(directory), {
      "local/custom": "sheet custom {}\n",
      "local/nested/other": "sheet other {}",
    });
  } finally {
    await rm(directory, { recursive: true });
  }
});

/** Catches silently omitting invalid local source names or unreadable directories. */
test("local example discovery rejects unsafe names and missing directories", async () => {
  const directory = await mkdtemp(join(tmpdir(), "adam-slides-local-"));
  try {
    await writeFile(join(directory, "Bad Name.adm2"), "sheet bad {}");
    await assert.rejects(build.readLocalExamples(directory), /name|key|selector/i);
    await assert.rejects(build.readLocalExamples(join(directory, "missing")), { code: "ENOENT" });
  } finally {
    await rm(directory, { recursive: true });
  }
});
