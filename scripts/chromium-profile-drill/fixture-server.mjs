// No real service or credential is contacted. Authentication is a fixed local
// fixture with a server-side revocation switch independent of browser storage.
import { createServer } from "node:http";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export function fixtureServer() {
  let revoked = false;
  const server = createServer((request, response) => {
    response.setHeader("Cache-Control", "no-store");
    if (request.method === "GET" && request.url === "/app.html") {
      response.setHeader("Content-Type", "text/html; charset=utf-8");
      response.end("<!doctype html><title>Chariox profile fixture</title><p>Deterministic local storage fixture</p>");
    } else if (request.method === "POST" && request.url === "/login") {
      revoked = false;
      response.setHeader("Set-Cookie", "chariox_fixture_auth=fixture-v1; HttpOnly; SameSite=Strict; Path=/; Max-Age=86400");
      response.end("ok");
    } else if (request.method === "POST" && request.url === "/revoke") {
      revoked = true;
      response.end("ok");
    } else if (request.method === "GET" && request.url === "/auth") {
      const authenticated = !revoked && (request.headers.cookie ?? "").split(/;\s*/).includes("chariox_fixture_auth=fixture-v1");
      response.setHeader("Content-Type", "application/json");
      response.end(JSON.stringify({ authenticated }));
    } else { response.writeHead(404); response.end(); }
    request.resume();
  });
  server.requestTimeout = 5000;
  server.headersTimeout = 5000;
  server.maxHeadersCount = 32;
  return server;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const server = fixtureServer();
  server.listen(8765, "127.0.0.1");
  process.on("SIGTERM", () => server.close(() => process.exit(0)));
}
