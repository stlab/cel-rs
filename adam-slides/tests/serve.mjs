import { createServer } from "node:http";
import { readFile, realpath } from "node:fs/promises";
import { extname, isAbsolute, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const prefix = "/project/docs/adam-slides/";
const mime = {
  ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript",
  ".css": "text/css", ".json": "application/json", ".wasm": "application/wasm",
};

/**
 * Returns whether a path names the root or one of its descendants by path components.
 * Complexity: O(p) time and space in path bytes.
 */
function isContained(root, file) {
  const within = relative(root, file);
  return within !== ".." && !within.startsWith(`..${sep}`) && !isAbsolute(within);
}

/**
 * Serves one request beneath the caller-owned static output; returns real 404s.
 * Rejects traversal and symlinks resolving outside that output with HTTP 403.
 * The caller owns the server and response lifetime.
 * Precondition: directory/symlink topology remains unchanged during each request.
 * Complexity: O(p + n) time and space in path bytes p and file bytes n, plus filesystem IO.
 * Example: createServer((request, response) => serve(directory, request, response)).
 */
export async function serve(root, request, response) {
  try {
    const url = new URL(request.url, "http://localhost");
    if (!url.pathname.startsWith(prefix)) {
      response.writeHead(404).end("Not found");
      return;
    }
    const name = decodeURIComponent(url.pathname.slice(prefix.length)) || "index.html";
    const file = resolve(root, name);
    if (!isContained(root, file)) {
      response.writeHead(403).end("Forbidden");
      return;
    }
    const canonicalRoot = await realpath(root);
    const canonicalFile = await realpath(file);
    if (!isContained(canonicalRoot, canonicalFile)) {
      response.writeHead(403).end("Forbidden");
      return;
    }
    const content = await readFile(canonicalFile);
    response.writeHead(200, { "Content-Type": mime[extname(file)] ?? "application/octet-stream" });
    response.end(content);
  } catch (error) {
    if (error.code === "ENOENT" || error.code === "EISDIR" || error instanceof URIError) {
      response.writeHead(404).end("Not found");
    } else {
      console.error(error);
      response.writeHead(500).end("Static server error");
    }
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = resolve(process.env.SLIDES_DIR ?? fileURLToPath(new URL("../dist", import.meta.url)));
  const server = createServer((request, response) => serve(root, request, response));
  server.listen(3419, "127.0.0.1", () => {
    console.log(`Adam slides: http://127.0.0.1:3419${prefix}`);
  });
}
