#!/usr/bin/env node
// MP-08/MP-10: timing-exact Drill E. The selected test artifact must be built
// from this checkout; this runner records identity, it does not attest provenance.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { createReadStream } from "node:fs";
import { mkdir, mkdtemp, readFile, realpath, rm, statfs, symlink, writeFile } from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { fileURLToPath } from "node:url";
import { assertConcurrencyArtifact, assertConcurrencyResources, concurrencyRustMinStackBytes, concurrencyTestSymbol } from "./lib/browser-controller-concurrency-preflight.mjs";

import { signalBrowserStateChild } from "./lib/browser-state-drill-process.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");
const options = {};
for (let i = 2; i < process.argv.length; i += 2) {
  assert.ok(["--test-binary", "--image", "--output", "--test-binary-sha256", "--memory-floor-gib"].includes(process.argv[i]), "MP-08/MP-10 unknown option");
  assert.ok(process.argv[i + 1] && !process.argv[i + 1].startsWith("--"), "MP-08/MP-10 missing option value");
  options[process.argv[i].slice(2)] = process.argv[i + 1];
}
for (const key of ["test-binary", "image", "output"]) assert.ok(options[key], `MP-08/MP-10 --${key} is required`);
assert.ok(path.isAbsolute(options["test-binary"]) && path.isAbsolute(options.output));
assert.ok(!options.output.startsWith(root + path.sep), "MP-08/MP-10 evidence must be outside the repository");
assert.match(options.image, /^sha256:[a-f0-9]{64}$/, "MP-08/MP-10 pin the engine-resolved image ID");
await mkdir(options.output, { recursive: true, mode: 0o700 });
const container = `chariox-ctlconc-${process.pid}-${Date.now()}`;
const mp_items = ["MP-08", "MP-10"];
const memoryFloorGiB = Number(options["memory-floor-gib"] ?? 4);
const receipt = { mp_items, startedAt: new Date().toISOString(), command: process.argv,
  image: options.image, container, controllerBoundMs: 5000, memoryFloorGiB,
  rustMinStackBytes: concurrencyRustMinStackBytes, commands: [], resources: [], cleanup: null,
  limitations: ["Local synthetic kernel clients and real headed controller; no provider, Web, hosted relay, signed release or fresh Path-1 acceptance."] };
const owned = new Set();
let interrupted = false;
for (const signal of ["SIGINT", "SIGTERM"]) process.once(signal, () => {
  interrupted = true;
  for (const child of owned) signalBrowserStateChild(child, "SIGTERM");
});
async function command(program, args, { timeout = 120000, env = process.env, log } = {}) {
  assert.ok(!interrupted, "MP-08/MP-10 interrupted");
  const record = { program, args, startedAt: new Date().toISOString() }; receipt.commands.push(record);
  const child = spawn(program, args, { cwd: root, env, stdio: ["ignore", "pipe", "pipe"], detached: true }); owned.add(child);
  let stdout = "", stderr = "";
  child.stdout.on("data", chunk => { stdout += chunk; }); child.stderr.on("data", chunk => { stderr += chunk; });
  const timer = setTimeout(() => { try { signalBrowserStateChild(child, "SIGTERM"); } catch {} }, timeout);
  const code = await new Promise((resolve, reject) => { child.once("error", reject); child.once("exit", resolve); });
  clearTimeout(timer); owned.delete(child); record.exitCode = code; record.finishedAt = new Date().toISOString();
  if (log) await writeFile(log, stdout + stderr, { mode: 0o600 });
  assert.equal(code, 0, `MP-08/MP-10 ${program} failed: ${stderr.slice(-3000)}${stdout.slice(-3000)}`);
  return { stdout, stderr };
}
const docker = args => command("docker", args);
async function resources() {
  const disk = await statfs("/");
  const memory = await readFile("/proc/meminfo", "utf8");
  const sample = { atMs: Date.now(), diskBytes: disk.bavail * disk.bsize,
    memAvailableKiB: Number(memory.match(/^MemAvailable:\s+(\d+)/m)?.[1]) };
  receipt.resources.push(sample);
  assertConcurrencyResources(sample, memoryFloorGiB);
}
let watchdog;
let started = false;
let transportDirectory;
try {
  await resources();
  watchdog = setInterval(() => { resources().catch(() => { interrupted = true; for (const child of owned) signalBrowserStateChild(child, "SIGTERM"); }); }, 5000);
  receipt.source = (await command("git", ["rev-parse", "HEAD"])).stdout.trim();
  receipt.tree = (await command("git", ["rev-parse", "HEAD^{tree}"])).stdout.trim();
  assert.equal((await command("git", ["status", "--porcelain"])).stdout.trim(), "", "MP-08/MP-10 requires clean source");
  const digest = createHash("sha256");
  for await (const chunk of createReadStream(options["test-binary"])) digest.update(chunk);
  receipt.testBinarySha256 = digest.digest("hex");
  const listing = (await command(options["test-binary"], [concurrencyTestSymbol, "--exact", "--ignored", "--list"])).stdout;
  assertConcurrencyArtifact({ sha256: receipt.testBinarySha256, expectedSha256: options["test-binary-sha256"], listing });
  receipt.testSymbol = concurrencyTestSymbol;
  receipt.testBinaryDigestPinned = options["test-binary-sha256"] !== undefined;
  await docker(["run", "-d", "--name", container, "--label", "io.chariox.drill=controller-concurrency",
    "--label", "io.chariox.lane=ctlconc", "--cpus=1", "--memory=2g", "--memory-swap=2g", "--pids-limit=512",
    "--security-opt", `seccomp=${path.join(root, "apps/kernel/slice-linux-docker/chromium-seccomp.json")}`,
    "--mount", `type=bind,source=${path.join(root, "apps/kernel/slice-linux-docker/docker")},target=/opt/chariox-slice,readonly`,
    "-p", "127.0.0.1::60222", "--entrypoint", "/usr/bin/sleep", options.image, "infinity"]);
  started = true;
  await docker(["exec", "-u", "slice", "-e", "CHARIOX_SLICE_DISPLAY_MODE=headed", container,
    "/opt/chariox-slice/slice-screen.sh", "start"]);
  await docker(["cp", path.join(root, "apps/cli/scripts/lib/browser-controller-concurrency-fixture.mjs"),
    `${container}:/tmp/browser-controller-concurrency-fixture.mjs`]);
  await docker(["exec", "-d", "-u", "slice", container, "node", "/tmp/browser-controller-concurrency-fixture.mjs"]);
  const mapping = (await docker(["port", container, "60222/tcp"])).stdout.trim();
  const endpoint = `http://${mapping}`;
  const deadline = Date.now() + 10000;
  while (true) {
    const state = await fetch(`${endpoint}/state`).then(r => r.json()).catch(() => null);
    if (["same", "other"].every(tab => state?.probes.some(p => p.tab === tab && p.kind === "ready"))) break;
    assert.ok(Date.now() < deadline, "MP-08/MP-10 page fixture readiness timeout");
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  receipt.inspect = JSON.parse((await docker(["inspect", container])).stdout)[0];
  transportDirectory = await mkdtemp(path.join(os.tmpdir(), "chariox-ctlconc-transport-"));
  await symlink(path.join(root, "apps/cli/scripts/lib/browser-controller-concurrency-transport.mjs"),
    path.join(transportDirectory, "docker"));
  const realDocker = await realpath((await command("which", ["docker"])).stdout.trim());
  const test = await command(options["test-binary"], [
    concurrencyTestSymbol,
    "--exact", "--ignored", "--nocapture"], { timeout: 60000, log: path.join(options.output, "test.log"),
    env: { ...process.env, RUST_MIN_STACK: String(concurrencyRustMinStackBytes),
      PATH: transportDirectory + path.delimiter + process.env.PATH,
      CHARIOX_CONCURRENCY_REAL_DOCKER: realDocker,
      CHARIOX_CONCURRENCY_TRANSPORT_RECEIPT: path.join(options.output, "transport.jsonl"),
      CHARIOX_CONCURRENCY_CONTAINER: container, CHARIOX_CONCURRENCY_FIXTURE: mapping } });
  await writeFile(path.join(options.output, "test.log"), test.stdout + test.stderr, { mode: 0o600 });
  receipt.probe = test.stdout.split("\n").filter(line => line.startsWith("{"))
    .map(line => { try { return JSON.parse(line); } catch { return null; } })
    .find(item => item?.schema === "chariox.controller_concurrency.v1");
  assert.equal(receipt.probe?.status, "GREEN", "MP-08/MP-10 missing exact acceptance probe");
  receipt.status = "GREEN";
} catch (error) {
  receipt.status = "RED"; receipt.failure = error.message; process.exitCode = 1;
} finally {
  clearInterval(watchdog);
  // Cleanup is permitted after interruption and is scoped to exact labels/name.
  interrupted = false;
  try {
    if (started) {
      const inspect = JSON.parse((await docker(["inspect", container])).stdout)[0];
      assert.equal(inspect.Name, "/" + container);
      assert.equal(inspect.Config.Labels["io.chariox.drill"], "controller-concurrency");
      assert.equal(inspect.Mounts.length, 1, "MP-08/MP-10 fixture must have only its read-only source mount");
      assert.equal(inspect.Mounts[0].Source, path.join(root, "apps/kernel/slice-linux-docker/docker"));
      assert.equal(inspect.Mounts[0].Destination, "/opt/chariox-slice");
      assert.equal(inspect.Mounts[0].RW, false);
      await docker(["rm", "-f", container]);
    }
    const names = (await docker(["ps", "-a", "--filter", `name=^/${container}$`, "--format", "{{.Names}}"])).stdout.trim();
    assert.equal(names, "");
    if (receipt.inspect) {
      const mapping = receipt.inspect.NetworkSettings.Ports["60222/tcp"][0];
      const stillListening = await fetch(`http://127.0.0.1:${mapping.HostPort}/state`, { signal: AbortSignal.timeout(1000) })
        .then(() => true, () => false);
      assert.equal(stillListening, false, "MP-08/MP-10 fixture listener survived container removal");
    }
    receipt.cleanup = { status: "GREEN", containerGone: true, listenerGone: true, processesGoneWithContainer: true,
      retainedImages: [options.image] };
    await resources();
  } catch (error) { receipt.cleanup = { status: "RED", error: error.message }; receipt.status = "RED"; process.exitCode = 1; }
  if (transportDirectory) await rm(transportDirectory, { recursive: true });
  receipt.finishedAt = new Date().toISOString();
  await writeFile(path.join(options.output, "result.json"), JSON.stringify(receipt, null, 2) + "\n", { mode: 0o600 });
  console.log(JSON.stringify({ mp_items, status: receipt.status, cleanup: receipt.cleanup, evidence: options.output }));
}
