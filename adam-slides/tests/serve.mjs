import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(process.env.SLIDES_DIR ?? fileURLToPath(new URL("../dist", import.meta.url)));
const prefix = "/project/docs/adam-slides/";
const mime = {
  ".html": "text/html", ".js": "text/javascript", ".mjs": "text/javascript",
  ".css": "text/css", ".json": "application/json", ".wasm": "application/wasm",
};

/**
 * Serves only files beneath the owned static output; returns real 404s.
 * Complexity: O(n) in requested file bytes.
 */
async function serve(request, response) {
  try {
    const url = new URL(request.url, "http://localhost");
    if (!url.pathname.startsWith(prefix)) {
      response.writeHead(404).end("Not found");
      return;
    }
    const name = decodeURIComponent(url.pathname.slice(prefix.length)) || "index.html";
    const file = resolve(root, name);
    const within = relative(root, file);
    if (within.startsWith("..") || isAbsolute(within)) {
      response.writeHead(403).end("Forbidden");
      return;
    }
    const content = await readFile(file);
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

const server = createServer(serve);
server.listen(3419, "127.0.0.1", () => {
  console.log(`Adam slides: http://127.0.0.1:3419${prefix}`);
});
