import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { mkdtemp, mkdir, chmod, lstat, readFile, realpath, rm, symlink, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { createRetainedRoomRuntime, stopOrDeleteRoomSlice, roomCleanupComplete, verifyRetainedRoomArchive } from "./room-provider-retention.mjs"

async function fixture(t) {
  const root = await mkdtemp(path.join(os.tmpdir(), "room-retention-test-"))
  t.after(() => rm(root, { recursive: true, force: true }))
  return root
}

test("provider runtime is private, durable, unique and records retention without credentials", async t => {
  const home = await fixture(t)
  const roots = await Promise.all([createRetainedRoomRuntime({ home }), createRetainedRoomRuntime({ home })])
  assert.notEqual(...roots)
  for (const root of roots) {
    assert.ok(root.startsWith(path.join(home, ".chariox", "dev", "provider-runtimes", "browser-computer") + path.sep))
    assert.equal((await lstat(root)).mode & 0o777, 0o700)
    const receipt = path.join(root, "RETENTION.json")
    assert.equal((await lstat(receipt)).mode & 0o777, 0o600)
    assert.equal(JSON.parse(await readFile(receipt, "utf8")).cleanup, "retain")
  }
})

test("existing public dedicated root is rejected without changing its permissions", async t => {
  const home = await fixture(t)
  const base = path.join(home, ".chariox", "dev", "provider-runtimes")
  await mkdir(base, { recursive: true })
  await chmod(base, 0o755)
  await assert.rejects(createRetainedRoomRuntime({ home }), /mode0700/)
  assert.equal((await lstat(base)).mode & 0o777, 0o755)
})

test("symlink in durable hierarchy is rejected without writing into its target", async t => {
  const home = await fixture(t)
  const target = await fixture(t)
  await mkdir(path.join(home, ".chariox", "dev"), { recursive: true })
  await symlink(target, path.join(home, ".chariox", "dev", "provider-runtimes"))
  await assert.rejects(createRetainedRoomRuntime({ home }), /real directory/)
  await assert.rejects(lstat(path.join(target, "browser-computer")), { code: "ENOENT" })
})

test("failed credential-bearing stop never falls back to deletion", async () => {
  let removed = false
  await assert.rejects(stopOrDeleteRoomSlice({ retain: true,
    stop: async () => { throw new Error("stop failed") }, remove: async () => { removed = true },
  }), /stop failed/)
  assert.equal(removed, false)
})

test("cleanup distinguishes retained credential state from removed synthetic state", () => {
  const common = { containerGone: true, fixtureWorkspaceRemoved: true, listenersReleased: true, plaintextSecretLeak: false }
  const retained = { ...common, providerStateRetained: true, homeVolumeRetained: true, volumeGone: false, tempRootRemoved: false }
  assert.equal(roomCleanupComplete(retained, true), true)
  assert.equal(roomCleanupComplete(retained, false), false)
  assert.equal(roomCleanupComplete({ ...retained, homeVolumeRetained: false }, true), false)
  assert.equal(roomCleanupComplete({ ...common, volumeGone: true, tempRootRemoved: true }, false), true)
})

async function savedFixture(t) {
  const root = await realpath(await fixture(t))
  const state = { id: "synthetic-state", source_slice_id: "synthetic-slice", image_ref: "synthetic-image",
    home_archive_path: path.join(root, "home.tar.zst"), manifest_path: path.join(root, "manifest.json"), size_bytes: 10 }
  await writeFile(state.home_archive_path, "0123456789", { mode: 0o600 })
  await writeFile(state.manifest_path, JSON.stringify(state), { mode: 0o600 })
  return state
}

test("unsafe archive permissions block original-volume removal preflight", async t => {
  const state = await savedFixture(t)
  await chmod(state.home_archive_path, 0o644)
  await assert.rejects(verifyRetainedRoomArchive(state), /remain private/)
})

test("mismatched saved generation blocks original-volume removal preflight", async t => {
  const state = await savedFixture(t)
  await writeFile(state.manifest_path, JSON.stringify({ ...state, source_slice_id: "other" }))
  await assert.rejects(verifyRetainedRoomArchive(state), /source_slice_id differs/)
})

test("archive symlink blocks preflight without touching the referenced file", async t => {
  const state = await savedFixture(t)
  const alias = state.home_archive_path + ".alias"
  await symlink(state.home_archive_path, alias)
  await assert.rejects(verifyRetainedRoomArchive({ ...state, home_archive_path: alias }), /symlink/)
  assert.equal(await readFile(state.home_archive_path, "utf8"), "0123456789")
})

test("invalid manifest does not expose its contents in the failure", async t => {
  const state = await savedFixture(t)
  await writeFile(state.manifest_path, "SYNTHETIC_PRIVATE_MARKER_NOT_JSON")
  await assert.rejects(verifyRetainedRoomArchive(state), error => {
    assert.equal(error.message.includes("SYNTHETIC_PRIVATE"), false)
    return error.message === "saved state manifest is not valid JSON"
  })
})

for (const logCanary of [false, true]) test(`real-provider failure retains protected state and audits logs (canary=${logCanary})`, async t => {
  const home = await realpath(await fixture(t))
  const bin = path.join(home, "bin")
  await mkdir(bin)
  const docker = path.join(bin, "docker")
  await writeFile(docker, `#!/usr/bin/env node
const fs = require("node:fs"), path = require("node:path");
fs.appendFileSync(path.join(process.env.HOME, "calls.jsonl"), JSON.stringify(process.argv.slice(2)) + "\\n");
if (process.argv[2] === "info") {
  const base = path.join(process.env.HOME, ".chariox/dev/provider-runtimes/browser-computer");
  for (const name of fs.readdirSync(base)) {
    const profile = path.join(base, name, "home/provider-profile");
    fs.mkdirSync(profile, {recursive:true, mode:0o700});
    fs.writeFileSync(path.join(profile, "synthetic-auth.json"), "SYNTHETIC_ACCOUNT_STATE", {mode:0o600});
    const large = path.join(profile, "protected-home-archive");
    fs.writeFileSync(large, "", {mode:0o600});
    fs.truncateSync(large, 65 * 1024 * 1024);
    if (process.env.CHARIOX_RETENTION_TEST_LOG_CANARY === "1") {
      const {runId} = JSON.parse(fs.readFileSync(path.join(base, name, "RETENTION.json"), "utf8"));
      const logs = path.join(base, name, "kernel-logs");
      fs.mkdirSync(logs, {mode:0o700});
      fs.writeFileSync(path.join(logs, "synthetic.log"), "web-" + runId + "-Grüße 世界", {mode:0o600});
    }
  }
  console.error("SYNTHETIC_PRIVATE_DIAGNOSTIC");
}
process.exit(1);
`, { mode: 0o700 })
  const script = new URL("../live-room-environment-pointer-click-drill.mjs", import.meta.url)
  const result = spawnSync(process.execPath, [fileURLToPath(script)], { timeout: 30_000, encoding: "utf8",
    env: { ...process.env, HOME: home, PATH: `${bin}${path.delimiter}${process.env.PATH}`,
      CHARIOX_ROOM_DRILL_FOCUS: "real-provider", CHARIOX_ROOM_DRILL_PROVIDER: "codex",
      CHARIOX_ROOM_DRILL_MODEL: "gpt-6.1-sol", CHARIOX_ROOM_DRILL_WEB_KEYBOARD: "1",
      CHARIOX_RETENTION_TEST_LOG_CANARY: logCanary ? "1" : "0", CARGO_TARGET_DIR: path.join(home, "missing-binaries") },
  })
  assert.equal(result.status, 1)
  assert.equal(`${result.stdout}${result.stderr}`.includes("SYNTHETIC_PRIVATE_DIAGNOSTIC"), false)
  const { readdir } = await import("node:fs/promises")
  const base = path.join(home, ".chariox/dev/provider-runtimes/browser-computer")
  const [name] = await readdir(base)
  const root = path.join(base, name)
  assert.equal(await readFile(path.join(root, "home/provider-profile/synthetic-auth.json"), "utf8"), "SYNTHETIC_ACCOUNT_STATE")
  assert.match(await readFile(path.join(root, "execution-record/failure.txt"), "utf8"), /SYNTHETIC_PRIVATE_DIAGNOSTIC/)
  const calls = (await readFile(path.join(home, "calls.jsonl"), "utf8")).trim().split("\n").map(JSON.parse)
  assert.equal(calls.some(args => args[0] === "volume" && args[1] === "rm"), false)
  const publicRoot = path.join(home, ".codex/evidence/browser-computer-use/computer-secret-room-e2e")
  const [stamp] = await readdir(publicRoot)
  const receipt = JSON.parse(await readFile(path.join(publicRoot, stamp, "provider-retention.json"), "utf8"))
  assert.equal(receipt.cleanup.providerStateRetained, true)
  assert.equal((await lstat(path.join(root, "execution-record/failure.txt"))).mode & 0o777, 0o600)
  assert.equal(JSON.parse(await readFile(path.join(root, "execution-record/cleanup.json"), "utf8")).plaintextSecretLeak, logCanary)
})
