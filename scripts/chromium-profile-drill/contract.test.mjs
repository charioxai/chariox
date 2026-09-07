import assert from "node:assert/strict";
import { once } from "node:events";
import test from "node:test";
import { fixtureServer } from "./fixture-server.mjs";
import { verifyInputs } from "./prepare.mjs";
import { cleanupResources } from "./resources.mjs";

test("browser fixture consumes the exact production base, snapshot, CA and packaged launcher inputs", verifyInputs);

test("fixture authentication requires its cookie and server revocation independently invalidates it", async () => {
  const server = fixtureServer();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const origin = `http://127.0.0.1:${server.address().port}`;
  try {
    assert.deepEqual(await (await fetch(`${origin}/auth`)).json(), { authenticated: false });
    const login = await fetch(`${origin}/login`, { method: "POST" });
    const cookie = login.headers.get("set-cookie");
    assert.match(cookie, /HttpOnly/);
    assert.match(cookie, /Max-Age=86400/);
    await login.text();
    const headers = { Cookie: cookie.split(";")[0] };
    assert.deepEqual(await (await fetch(`${origin}/auth`, { headers })).json(), { authenticated: true });
    await (await fetch(`${origin}/revoke`, { method: "POST" })).text();
    assert.deepEqual(await (await fetch(`${origin}/auth`, { headers })).json(), { authenticated: false });
    assert.equal((await fetch(`${origin}/unknown`)).status, 404);
  } finally { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); }
});

test("cleanup checks ownership again and removes only the identified fixture resources", () => {
  const owner = { id: "a".repeat(24) };
  const container = "b".repeat(64);
  const image = `sha256:${"c".repeat(64)}`;
  const volume = `chariox-chromium-${owner.id}-source`;
  const calls = [];
  cleanupResources(owner, args => {
    calls.push(args);
    if (args[0] === "ps") return container;
    if (args[0] === "volume" && args[1] === "ls") return volume;
    if (args[0] === "image" && args[1] === "ls") return image;
    if (args.includes("inspect")) return owner.id;
    return "";
  });
  assert.deepEqual(calls.filter(args => args.includes("rm")), [
    ["rm", "--force", container], ["volume", "rm", volume], ["image", "rm", image],
  ]);
  assert.ok(!calls.some(args => args.includes("prune")));
  const rejected = [];
  assert.throws(() => cleanupResources(owner, args => {
    rejected.push(args);
    return args[0] === "ps" ? container : "different-owner";
  }));
  assert.ok(!rejected.some(args => args.includes("rm")));
});
