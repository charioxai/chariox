import assert from "node:assert/strict";
import { once } from "node:events";
import test from "node:test";
import { fixtureServer } from "./fixture-server.mjs";
import { verifyInputs, source, fixtureSource } from "./prepare.mjs";
import { restoreDockerShim, restoreScript } from "./restore.mjs";
import { spawnSync } from "node:child_process";
import { drillEnvironment } from "./environment.mjs";
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { cleanupResources } from "./resources.mjs";

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
    if (expected) assert.throws(() => verifyInputs({ sourceRoot, fixtureRoot }), expected);
    else verifyInputs({ sourceRoot, fixtureRoot });
  } finally { rmSync(root, { recursive: true, force: true }); }
}

for (const [label, area, file, mutate, expected] of [
  ["launcher bytes", "source", "docker/slice-screen.sh", s => s + "\n# changed launcher\n", /hash drifted/],
  ["actionability dependency bytes", "source", "docker/browser-controller-actionability.mjs", s => s + "\n// MP-08 changed dependency\n", /hash drifted/],
  ["interaction dependency bytes", "source", "docker/browser-controller-interactions.mjs", s => s + "\n// MP-08 changed dependency\n", /hash drifted/],
  ["geometry dependency bytes", "source", "docker/browser-controller-geometry.mjs", s => s + "\n// MP-08 changed dependency\n", /hash drifted/],
  ["commented launcher COPY", "source", "docker/Dockerfile", s => s.replace("COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/slice-screen.sh", "# COPY --chown=slice:slice apps/kernel/slice-linux-docker/docker/slice-screen.sh"), /omits slice-screen/],
  ["missing apt package", "fixture", "Dockerfile", s => s.replace("chromium-sandbox ", ""), /apt package selection/],
  ["launcher COPY", "source", "docker/Dockerfile", s => s.replace("docker/slice-screen.sh /opt/chariox-slice/slice-screen.sh", "docker/missing.sh /opt/chariox-slice/slice-screen.sh"), /omits slice-screen/],
  ["input hash", "fixture", "inputs.lock.json", s => s.replace(/[a-f0-9]{64}/, "0".repeat(64)), /hash drifted/],
  ["image digest", "source", "docker/Dockerfile", s => s.replace(/d649[a-f0-9]{60}/, "0".repeat(64)), /inputs drifted/],
  ["CA checksum", "source", "docker/Dockerfile", s => s.replace(/a341[a-f0-9]{60}/, "0".repeat(64)), /inputs drifted/],
  ["runtime CA copy", "source", "docker/Dockerfile", s => { const i = s.lastIndexOf("COPY --from=ca-bundle"); return s.slice(0, i) + "# " + s.slice(i); }, /CA bundle copy/],
  ["production apt package", "source", "docker/Dockerfile", s => s.replace("    chromium-sandbox ", "    chromium-sandbox=0 "), /apt package drifted/],
  ["CA copy", "fixture", "Dockerfile", s => s.replace("COPY --from=ca-bundle", "# COPY --from=ca-bundle"), /CA bundle copy/],
  ["apt snapshot", "source", "docker/Dockerfile", s => s.replaceAll("20260701T000000Z", "20260702T000000Z"), /inputs drifted/],
  ["apt validity", "fixture", "Dockerfile", s => s.replace("Acquire::Check-Valid-Until=false", "Acquire::Check-Valid-Until=true"), /apt setup drifted/],
  ["apt package", "fixture", "Dockerfile", s => s.replace("chromium-sandbox curl", "chromium-sandbox=0 curl"), /apt package/],
  ["restore provisioner bytes", "source", "provision-linux-docker-slice.sh", s => s.replace("prepare_home_volume() {", "prepare_home_volume() {\n# changed restore"), /hash drifted/],
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
  if (process.platform !== "linux") {
    assert.throws(() => drillEnvironment({ CHARIOX_CHROMIUM_DRILL_ENVIRONMENT: "builder" }));
    return;
  }
  assert.equal(drillEnvironment({ CHARIOX_CHROMIUM_DRILL_ENVIRONMENT: "builder" }), "builder");
  const hosted = { GITHUB_ACTIONS: "true", RUNNER_ENVIRONMENT: "github-hosted", GITHUB_REPOSITORY: "charioxai/chariox" };
  assert.equal(drillEnvironment(hosted), "github-hosted");
  assert.throws(() => drillEnvironment({}));
  assert.throws(() => drillEnvironment({ ...hosted, CHARIOX_CHROMIUM_DRILL_ENVIRONMENT: "builder" }));
  assert.throws(() => drillEnvironment({ CHARIOX_CHROMIUM_DRILL_ENVIRONMENT: "unknown" }));
});

test("restore adapter consumes current guarded production functions and rejects a missing section", () => {
  const sourceText = readFileSync(join(source, "provision-linux-docker-slice.sh"), "utf8");
  const script = restoreScript(sourceText);
  assert.match(script, /created_archive_label.*!=.*archive_identity/);
  assert.match(script, /created_token_label.*!=.*initialization_token/);
  assert.match(script, /if restore_saved_home_volume; then/);
  assert.ok(script.endsWith("prepare_home_volume"));
  assert.doesNotMatch(script, /restore-migration-home|build_image|import_provider_auth/);
  assert.throws(() => restoreScript(sourceText.replace("prepare_home_volume() {", "missing_home_function() {")), /prepare_home_volume function is missing/);
});

test("restore Docker shim preserves production helper options without duplicating network or resource limits", () => {
  const label = `io.chariox.chromium-drill=${"a".repeat(24)}`;
  // Intercept only the final exec; exercise the actual shell dispatch without
  // creating Docker resources during the contract test.
  const invoke = args => {
    const result = spawnSync("bash", ["-c",
      `exec() { printf '%s\\0' "$@"; exit 0; }\n${restoreDockerShim(label)}`,
      "--", ...args], { encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
    return result.stdout.split("\0").slice(0, -1);
  };
  const helper = ["--name", "restore-helper", "--user", "root", "--memory", "512m",
    "--cpus", "1", "--pids-limit", "64", "--network", "none", "--label",
    "io.chariox.home-restore-helper=restore-helper", "-v", "home:/home-dst", "image", "sleep", "infinity"];
  assert.deepEqual(invoke(["create", ...helper]),
    ["/usr/bin/docker", "create", "--memory-swap", "512m", "--label", label, ...helper]);
  assert.deepEqual(invoke(["volume", "create", "--label", "archive=identity", "home"]),
    ["/usr/bin/docker", "volume", "create", "--label", label, "--label", "archive=identity", "home"]);
  assert.deepEqual(invoke(["exec", "-i", "restore-helper", "tar", "--zstd", "-xf", "-"]),
    ["/usr/bin/docker", "exec", "-i", "restore-helper", "tar", "--zstd", "-xf", "-"]);
  assert.throws(() => restoreDockerShim("invalid-owner"));
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

test("unrelated provisioner changes do not invalidate consumed restore functions", () => {
  mutatedInput("source", "provision-linux-docker-slice.sh", s => s + "\n# unrelated function change\n");
});
