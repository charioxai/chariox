import { test } from "node:test"
import assert from "node:assert/strict"
import { PRIVATE_ROOT, PRIVATE_ENVIRONMENT, verifyProtectedCaptureLayout, requireSupportedHomeEntries } from "../apps/kernel/slice-linux-docker/protected-layout.mjs"

const digest = `sha256:${"a".repeat(64)}`
function fixture() {
  const receipt = { version: 1, containerId: "synthetic", imageId: digest, baseImageId: digest,
    privateHostRoot: "/protected/synthetic", homeVolume: "synthetic-home" }
  const inspect = { Id: "synthetic", Image: digest, Mounts: [
    { Type: "bind", Source: receipt.privateHostRoot, Destination: PRIVATE_ROOT, RW: true },
    { Type: "volume", Name: receipt.homeVolume, Destination: "/home/slice", RW: true },
    { Type: "bind", Source: `${receipt.privateHostRoot}/nssdb`, Destination: "/home/slice/.local/share/pki/nssdb", RW: true },
  ], Config: { Env: ["HOME=/home/slice", ...Object.entries(PRIVATE_ENVIRONMENT).map(([k,v]) => `${k}=${v}`)] } }
  return { inspect, receipt }
}
const verify = ({ inspect, receipt }, trusted = new Set([digest])) => verifyProtectedCaptureLayout(inspect, receipt, trusted)
test("verified actual topology accepts only the protected roots", () => assert.deepEqual(verify(fixture()), { privateHostRoot: "/protected/synthetic", homeVolume: "synthetic-home" }))
test("forged labels cannot replace mounts or trusted base proof", () => {
  const f = fixture(); f.inspect.Config.Labels = { protected: "true" }; f.inspect.Mounts = []
  assert.throws(() => verify(f)); assert.throws(() => verify(fixture(), new Set()))
})
test("wrong source, nested overlay and missing NSS separation refuse", () => {
  for (const change of [f => f.inspect.Mounts[0].Source = "/different", f => f.inspect.Mounts.pop(), f => f.inspect.Mounts.push({Destination: `${PRIVATE_ROOT}/kernel`, Type: "volume"})]) {
    const f = fixture(); change(f); assert.throws(() => verify(f))
  }
})
test("token environment, duplicate env and unsafe host path refuse without secret output", () => {
  for (const change of [f => f.inspect.Config.Env.push("TOKEN=synthetic-private-sentinel"), f => f.inspect.Config.Env.push("HOME=/other"), f => f.receipt.privateHostRoot = "/protected/../other"]) {
    const f = fixture(); change(f); assert.throws(() => verify(f), e => !e.message.includes("sentinel"))
  }
})
test("known credential roots refuse while browser/workspace data stays supported", () => {
  for (const path of [".codex/auth.json", ".chariox/kernels/id/identity.json", ".ssh/key", "../private"]) assert.throws(() => requireSupportedHomeEntries([path]))
  assert.doesNotThrow(() => requireSupportedHomeEntries([".chariox/browser/chromium/Cookies", "Downloads/synthetic.txt"]))
})

import { mkdtempSync, rmSync, symlinkSync } from "node:fs"
import { join } from "node:path"
import { preparePrivateHostRoot } from "../apps/kernel/slice-linux-docker/protected-host-root.mjs"
test("private host roots never overwrite an existing slice or repair a missing root", () => {
  // tmpdir may have a writable ancestor, which deliberately fails the production
  // contract. Use the owned non-writable home ancestor for this synthetic test.
  const root = mkdtempSync(join(process.env.HOME, ".chariox-layout-test-"))
  try {
    const uid = process.getuid()
    const path = preparePrivateHostRoot(root, "synthetic", uid, true)
    assert.equal(preparePrivateHostRoot(root, "synthetic", uid, false), path)
    assert.throws(() => preparePrivateHostRoot(root, "synthetic", uid, true))
    assert.throws(() => preparePrivateHostRoot(root, "missing", uid, false))
    symlinkSync(path, join(root, "alias"))
    assert.throws(() => preparePrivateHostRoot(root, "alias", uid, false))
  } finally { rmSync(root, { recursive: true }) }
})

import { existsSync, lstatSync, writeFileSync, readFileSync } from "node:fs"
import { streamArchiveToProtectedSink } from "../apps/kernel/slice-linux-docker/protected-archive-stream.mjs"
test("archive streams only into protected durable sink and preserves prior generations", async () => {
  const root = mkdtempSync(join(process.env.HOME, ".chariox-layout-stream-test-"))
  try {
    const previous = join(root, "previous.tar.zst")
    writeFileSync(previous, "synthetic prior generation", {mode: 0o600})
    const path = join(root, "next.tar.zst")
    const result = await streamArchiveToProtectedSink({command: process.execPath,
      args: ["-e", "process.stdout.write('synthetic browser data')"], path, reserveBytes: 0})
    assert.equal(result.sizeBytes, 22)
    assert.equal(lstatSync(path).mode & 0o777, 0o600)
    assert.equal(readFileSync(previous, "utf8"), "synthetic prior generation")
    await assert.rejects(streamArchiveToProtectedSink({command: process.execPath,
      args: ["-e", "process.stdout.write('synthetic interrupted data');process.exit(1)"],
      path: join(root, "failed.tar.zst"), reserveBytes: 0}))
    assert.equal(existsSync(join(root, "failed.tar.zst")), false)
    assert.equal(readFileSync(previous, "utf8"), "synthetic prior generation")
    await assert.rejects(streamArchiveToProtectedSink({command: process.execPath,
      args: ["-e", "process.stdout.write('synthetic too-large data')"],
      path: join(root, "oversize.tar.zst"), maxBytes: 2, reserveBytes: 0}))
    assert.equal(existsSync(join(root, "oversize.tar.zst")), false)
  } finally { rmSync(root, {recursive: true}) }
})

import { writeProtectedLayoutReceipt, readProtectedLayoutReceipt, requireRetainedRuntimeIdentity } from "../apps/kernel/slice-linux-docker/protected-layout-store.mjs"
import { chmodSync } from "node:fs"
test("trusted host receipts and retained identity cannot be replaced by labels or missing files", () => {
  const root = mkdtempSync(join(process.env.HOME, ".chariox-layout-receipt-test-"))
  try {
    const receipt = {version: 1, sliceId: "synthetic", containerId: "synthetic-container"}
    writeProtectedLayoutReceipt(root, "synthetic", receipt)
    assert.deepEqual(readProtectedLayoutReceipt(root, "synthetic"), receipt)
    assert.throws(() => readProtectedLayoutReceipt(root, "../synthetic"))
    const privateRoot = preparePrivateHostRoot(root, "private", process.getuid(), true)
    const identity = "kernel/state/daemon/identity.json"
    assert.throws(() => requireRetainedRuntimeIdentity(privateRoot, [identity], process.getuid()))
    writeFileSync(join(privateRoot, identity), "synthetic sentinel, no private key", {mode: 0o600})
    assert.doesNotThrow(() => requireRetainedRuntimeIdentity(privateRoot, [identity], process.getuid()))
    chmodSync(join(privateRoot, identity), 0o644)
    assert.throws(() => requireRetainedRuntimeIdentity(privateRoot, [identity], process.getuid()))
  } finally { rmSync(root, {recursive: true}) }
})
