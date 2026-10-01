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
import { mkdirSync, chmodSync } from "node:fs"
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

import { recordManagedImageProof, requireManagedImageProof } from "../apps/kernel/slice-linux-docker/protected-image-proof.mjs"
test("managed image proof binds actual immutable image ID to signed source context", () => {
  const root = mkdtempSync(join(process.env.HOME, ".chariox-layout-image-test-"))
  try {
    const source = `sha256:${"b".repeat(64)}`
    const image = {Id: digest, Config: {User: "slice"}, RootFS: {Layers: [`sha256:${"c".repeat(64)}`]}}
    assert.throws(() => requireManagedImageProof(root, source, digest))
    recordManagedImageProof(root, source, image)
    assert.equal(requireManagedImageProof(root, source, digest), digest)
    assert.throws(() => requireManagedImageProof(root, `sha256:${"d".repeat(64)}`, digest))
    assert.throws(() => recordManagedImageProof(root, "self-asserted", image))
    assert.throws(() => recordManagedImageProof(root, source, {...image, Config: {User: "root"}}))
  } finally { rmSync(root, {recursive: true}) }
})

import { createManagedLayoutController } from "../apps/kernel/slice-linux-docker/protected-managed-layout.mjs"
test("fresh controller binds exact image/mount receipt and refuses missing same-slice identity", () => {
  const parent = mkdtempSync(join(process.env.HOME, ".chariox-layout-controller-test-"))
  try {
    const root = join(parent, "durable")
    const source = `sha256:${"b".repeat(64)}`
    const environment = {CHARIOX_SLICE_NAME: "chariox-slice-synthetic", CHARIOX_SLICE_ID: "slice-synthetic"}
    let info = null
    const docker = args => {
      if (args[0] === "ps") return {status: 0, stdout: info ? `${environment.CHARIOX_SLICE_NAME}\n` : ""}
      if (args[0] === "volume") return {status: 0, stdout: ""}
      if (args[0] === "container") return {status: 0, stdout: JSON.stringify([info])}
      throw new Error("unexpected synthetic Docker operation")
    }
    const controller = createManagedLayoutController({root, sourceDigest: source, docker, dataOwner: process.getuid()})
    assert.throws(() => controller.prepare("provision", {...environment, CHARIOX_SLICE_NAME: "../foreign"}))
    const privateRoot = controller.prepare("provision", environment)
    assert.throws(() => controller.prepare("provision", environment), "existing retained root cannot be silently initialized again")
    recordManagedImageProof(controller.imageRoot, source, {Id: digest, Config: {User: "slice"}, RootFS: {Layers: [digest]}})
    const directory = join(privateRoot, "kernel/kernels/synthetic-kernel")
    mkdirSync(directory, {mode: 0o700})
    const identity = join(directory, "identity.json")
    writeFileSync(identity, "synthetic identity sentinel, no private key", {mode: 0o600})
    writeFileSync(join(privateRoot, "kernel/kernels/registry.json"), "synthetic registry sentinel", {mode: 0o600})
    info = fixture().inspect
    info.Id = "synthetic-container"
    info.Mounts[0].Source = privateRoot
    info.Mounts[1].Name = `${environment.CHARIOX_SLICE_NAME}-home`
    info.Mounts[2].Source = `${privateRoot}/nssdb`
    environment.CHARIOX_SLICE_PRIVATE_HOST_ROOT = privateRoot
    controller.complete(environment)
    assert.equal(controller.preflight(environment.CHARIOX_SLICE_NAME).privateHostRoot, privateRoot)
    assert.equal(controller.prepare("recover", environment), privateRoot)
    const foreignOwnerController = createManagedLayoutController({root, sourceDigest: source, docker, dataOwner: process.getuid() + 1})
    assert.throws(() => foreignOwnerController.preflight(environment.CHARIOX_SLICE_NAME))
    const priorSource = info.Mounts[0].Source
    info.Mounts[0].Source = join(parent, "foreign-root")
    assert.throws(() => controller.preflight(environment.CHARIOX_SLICE_NAME))
    info.Mounts[0].Source = priorSource
    info.Config.Env.push("TOKEN=synthetic-private-sentinel")
    assert.throws(() => controller.preflight(environment.CHARIOX_SLICE_NAME), e => !e.message.includes("sentinel"))
    info.Config.Env.pop()
    rmSync(identity)
    assert.throws(() => controller.prepare("recover", environment))
    assert.equal(existsSync(identity), false, "missing identity must not be generated")
    assert.equal(existsSync(privateRoot), true, "retained private root must remain")
  } finally { rmSync(parent, {recursive: true}) }
})

import { retainFreshIdentity } from "../apps/kernel/slice-linux-docker/protected-identity-retention.mjs"
test("first-use retention refuses missing or invalid synthetic identity without starting a runtime", () => {
  const parent = mkdtempSync(join(process.env.HOME, ".chariox-retention-metadata-test-"))
  try {
    const privateRoot = preparePrivateHostRoot(parent, "private-retention", process.getuid(), true)
    const backupRoot = join(parent, "backups")
    mkdirSync(backupRoot, {mode: 0o700})
    assert.throws(() => retainFreshIdentity({privateRoot, backupRoot, sliceId: "synthetic", dataOwner: process.getuid()}))
    assert.equal(existsSync(join(backupRoot, "synthetic")), false)
    const kernel = join(privateRoot, "kernel/kernels/synthetic-kernel")
    mkdirSync(kernel, {mode: 0o700})
    writeFileSync(join(kernel, "identity.json"), JSON.stringify({daemon_id: "synthetic-kernel", relay_public_key: "invalid synthetic sentinel", relay_private_key: "not a key"}), {mode: 0o600})
    writeFileSync(join(privateRoot, "kernel/kernels/registry.json"), "synthetic registry", {mode: 0o600})
    writeFileSync(join(privateRoot, "kernel/machine/identity.json"), "synthetic machine", {mode: 0o600})
    assert.throws(() => retainFreshIdentity({privateRoot, backupRoot, sliceId: "synthetic", dataOwner: process.getuid()}))
    assert.equal(existsSync(join(backupRoot, "synthetic.json")), false, "invalid identity cannot publish first-use proof")
    assert.equal(existsSync(join(kernel, "identity.json")), true, "original identity remains untouched")
    assert.throws(() => retainFreshIdentity({privateRoot, backupRoot, sliceId: "synthetic", dataOwner: process.getuid()}), "interrupted backup must not be replaced")
  } finally { rmSync(parent, {recursive: true}) }
})
