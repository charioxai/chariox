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
