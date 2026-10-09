import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { prepareDeck } from "./build.mjs";

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
