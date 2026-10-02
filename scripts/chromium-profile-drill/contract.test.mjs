import assert from "node:assert/strict";
import { once } from "node:events";
import test from "node:test";
import { fixtureServer } from "./fixture-server.mjs";
import { verifyInputs, source, fixtureSource } from "./prepare.mjs";
import { drillEnvironment } from "./environment.mjs";
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { cleanupResources } from "./resources.mjs";
import { manifestFromLock } from "./controller.mjs";

test("browser fixture consumes the exact production base, snapshot, CA and packaged launcher inputs", () => verifyInputs());

function mutatedInput(area, file, mutate, expected) {
  const root = mkdtempSync(join(tmpdir(), "chariox-chromium-contract."));
  try {
    const sourceRoot = join(root, "source");
    const fixtureRoot = join(root, "fixture");
    // Copy only public source inputs, never runtime state or account data.
    cpSync(source, sourceRoot, { recursive: true });
    cpSync(fixtureSource, fixtureRoot, { recursive: true });
    const path = join(area === "source" ? sourceRoot : fixtureRoot, file);
    writeFileSync(path, mutate(readFileSync(path, "utf8")));
    assert.throws(() => verifyInputs({ sourceRoot, fixtureRoot }), expected);
  } finally { rmSync(root, { recursive: true, force: true }); }
}

for (const [label, area, file, mutate, expected] of [
  ["launcher bytes", "source", "docker/slice-screen.sh", s => s + "\n# changed launcher\n", /hash drifted/],
  ["commented launcher COPY", "source", "docker/Dockerfile", s => s.replace("COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/slice-screen.sh", "# COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/slice-screen.sh"), /omits slice-screen/],
  ["missing apt package", "fixture", "Dockerfile", s => s.replace("chromium-sandbox ", ""), /apt package selection/],
  ["launcher COPY", "source", "docker/Dockerfile", s => s.replace("docker/slice-screen.sh /opt/chariox-slice/slice-screen.sh", "docker/missing.sh /opt/chariox-slice/slice-screen.sh"), /omits slice-screen/],
  ["input hash", "fixture", "inputs.lock.json", s => s.replace(/[a-f0-9]{64}/, "0".repeat(64)), /hash drifted/],
  ["image digest", "source", "docker/Dockerfile", s => s.replace(/d649[a-f0-9]{60}/, "0".repeat(64)), /inputs drifted/],
  ["CA checksum", "source", "docker/Dockerfile", s => s.replace(/a341[a-f0-9]{60}/, "0".repeat(64)), /inputs drifted/],
  ["CA copy", "fixture", "Dockerfile", s => s.replace("COPY --from=ca-bundle", "# COPY --from=ca-bundle"), /CA bundle copy/],
  ["apt snapshot", "source", "docker/Dockerfile", s => s.replaceAll("20260701T000000Z", "20260702T000000Z"), /inputs drifted/],
  ["apt validity", "fixture", "Dockerfile", s => s.replace("Acquire::Check-Valid-Until=false", "Acquire::Check-Valid-Until=true"), /apt setup drifted/],
  ["apt package", "fixture", "Dockerfile", s => s.replace("chromium-sandbox curl", "chromium-sandbox=0 curl"), /apt package/],
  ["seccomp bytes", "source", "chromium-seccomp.json", s => s.replace("SCMP_ACT_ERRNO", "SCMP_ACT_ALLOW"), /hash drifted/],
]) test(`input contract rejects mutated ${label}`, () => mutatedInput(area, file, mutate, expected));

test("stale production helper expectation reproduces the old failure; probe is visibly test-only", () => {
  const production = readFileSync(join(source, "docker/Dockerfile"), "utf8");
  const stale = () => assert.ok(production.includes("docker/chromium-sandbox-probe.mjs /opt/chariox-slice/chromium-sandbox-probe.mjs"), "production image omits chromium-sandbox-probe.mjs");
  assert.throws(stale, /production image omits chromium-sandbox-probe/);
  assert.match(readFileSync(join(fixtureSource, "Dockerfile"), "utf8"), /COPY chromium-sandbox-probe.mjs \/opt\/chariox-drill\//);
  verifyInputs();
});

test("builder invocation is explicit and cannot claim hosted evidence", () => {
  assert.equal(drillEnvironment({ CHARIOX_CHROMIUM_DRILL_ENVIRONMENT: "builder" }), "builder");
  const hosted = { GITHUB_ACTIONS: "true", RUNNER_ENVIRONMENT: "github-hosted", GITHUB_REPOSITORY: "charioxai/chariox" };
  assert.equal(drillEnvironment(hosted), "github-hosted");
  assert.throws(() => drillEnvironment({}));
  assert.throws(() => drillEnvironment({ ...hosted, CHARIOX_CHROMIUM_DRILL_ENVIRONMENT: "builder" }));
  assert.throws(() => drillEnvironment({ CHARIOX_CHROMIUM_DRILL_ENVIRONMENT: "unknown" }));
});

test("controller harness rejects missing or ambiguous dependency pins", () => {
  const pins = ["tokio", "tokio-tungstenite", "futures-util", "serde_json", "base64"];
  const lock = pins.map(name => `[[package]]\nname = "${name}"\nversion = "1.2.3"\n`).join("");
  const manifest = manifestFromLock(lock);
  assert.match(manifest, /tokio = \{ version = "=1.2.3", features/);
  assert.throws(() => manifestFromLock(lock.replace('name = "tokio"', 'name = "missing"')));
  assert.throws(() => manifestFromLock(lock + '[[package]]\nname = "tokio"\nversion = "1.2.4"\n'));
});

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
