import assert from "node:assert/strict";
import { once } from "node:events";
import { createServer } from "node:http";
import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import test from "node:test";
import { serve } from "./tests/serve.mjs";

/**
 * Runs a case against an owned temporary site and loopback server, then closes both.
 * Propagates fixture/request failures after cleanup.
 * Complexity: the case's filesystem/request costs and stored fixture bytes.
 */
async function withSite(run) {
  const directory = await mkdtemp(join(tmpdir(), "adam-slides-server-"));
  const root = join(directory, "site");
  let server;
  try {
    await mkdir(root);
    server = createServer((request, response) => serve(root, request, response));
    server.listen(0, "127.0.0.1");
    await once(server, "listening");
    const base = `http://127.0.0.1:${server.address().port}/project/docs/adam-slides/`;
    await run({ directory, root, base });
  } finally {
    if (server?.listening) {
      await new Promise((resolve, reject) => {
        server.close((error) => error ? reject(error) : resolve());
      });
    }
    await rm(directory, { recursive: true, force: true });
  }
}

/** Checks successful content/MIME delivery and visible missing or escaped paths. */
test("static requests preserve the hosting prefix and valid in-root names", async () => {
  await withSite(async ({ root, base }) => {
    await writeFile(join(root, "index.html"), "<h1>Owned site</h1>");
    await writeFile(join(root, "..notes"), "Still inside the site");
    await writeFile(join(root, "example.mjs"), "export const example = true;");
    const index = await fetch(base);
    assert.equal(index.status, 200);
    assert.equal(index.headers.get("content-type"), "text/html");
    assert.equal(await index.text(), "<h1>Owned site</h1>");
    const notes = await fetch(`${base}..notes`);
    assert.equal(notes.status, 200);
    assert.equal(await notes.text(), "Still inside the site");
    const module = await fetch(`${base}example.mjs`);
    assert.equal(module.headers.get("content-type"), "text/javascript");
    await module.text();
    for (const [url, status] of [
      [new URL("/elsewhere", base), 404],
      [`${base}missing.html`, 404],
      [`${base}%invalid`, 404],
      [`${base}..%2Foutside`, 403],
    ]) {
      const response = await fetch(url);
      assert.equal(response.status, status);
      await response.text();
    }
  });
});

/** Checks physical containment while retaining legitimate aliases inside the owned site. */
test("static requests reject escaped symlinks but serve in-root aliases", async () => {
  await withSite(async ({ directory, root, base }) => {
    await writeFile(join(directory, "outside.txt"), "Not owned by the site");
    await writeFile(join(root, "inside.txt"), "Owned content");
    await symlink(join(directory, "outside.txt"), join(root, "escape.txt"));
    await symlink(join(root, "inside.txt"), join(root, "alias.txt"));
    const escaped = await fetch(`${base}escape.txt`);
    assert.equal(escaped.status, 403);
    assert.equal(await escaped.text(), "Forbidden");
    const alias = await fetch(`${base}alias.txt`);
    assert.equal(alias.status, 200);
    assert.equal(await alias.text(), "Owned content");
  });
});
