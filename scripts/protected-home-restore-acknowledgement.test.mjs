import test from "node:test"
import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { mkdtempSync, chmodSync, mkdirSync, rmSync } from "node:fs"
import { join } from "node:path"
import { createHomeGenerationStore } from "../apps/kernel/slice-linux-docker/protected-home-generation.mjs"
import { createManagedLayoutController } from "../apps/kernel/slice-linux-docker/protected-managed-layout.mjs"
import { readProtectedLayoutReceipt, writeProtectedLayoutReceipt } from "../apps/kernel/slice-linux-docker/protected-layout-store.mjs"

const container = "chariox-slice-acknowledged"
const imageId = `sha256:${"a".repeat(64)}`
const sourceDigest = `sha256:${"e".repeat(64)}`

// Kernel-side resolution is durable before the broker hears about it. These
// cases cover a lost or failed acknowledgement followed by another restore.
test("a lost restore acknowledgement is retried idempotently before the next restore", () => {
  const root = mkdtempSync(join(process.env.HOME, ".chariox-restore-acknowledgement-test-"))
  chmodSync(root, 0o711)
  try {
    const receipts = join(root, "receipts")
    const origins = join(root, "capture-origins")
    mkdirSync(receipts, {mode: 0o700})
    mkdirSync(origins, {mode: 0o700})
    const controller = () => createManagedLayoutController({root, sourceDigest, dataOwner: process.getuid(),
      docker: () => { throw new Error("restore acknowledgement must not invoke Docker") }})
    const retainOrigin = (containerId, homeVolume, digest) => {
      const key = createHash("sha256").update(`${containerId}\0${digest}`).digest("hex")
      writeProtectedLayoutReceipt(origins, key, {version: 1, sliceId: key, container, containerId, homeVolume, digest})
      return readProtectedLayoutReceipt(origins, key)
    }
    const restore = (store, {oldContainerId, oldHomeVolume, digest, containerId}) => {
      const record = store.begin({container, oldHomeVolume, oldContainerId, archiveDigest: digest, imageId,
        targetOrigin: retainOrigin(oldContainerId, oldHomeVolume, digest)})
      const parameters = {container, token: record.token, volume: record.newHomeVolume, digest}
      store.complete(parameters)
      writeProtectedLayoutReceipt(receipts, container, {version: 1, sliceId: container, containerId,
        imageId, homeVolume: record.newHomeVolume})
      store.publish(parameters)
      return record
    }

    const firstDigest = "b".repeat(64)
    const first = restore(createHomeGenerationStore(root), {oldContainerId: "original-container",
      oldHomeVolume: `${container}-home`, digest: firstDigest, containerId: "first-container"})
    // Kernel crash after durable resolution: no acknowledgement reached the broker.
    const restarted = createHomeGenerationStore(root)
    assert.equal(restarted.read(container).phase, "published")
    assert.throws(() => restarted.begin({container, oldHomeVolume: first.newHomeVolume,
      oldContainerId: "first-container", archiveDigest: "c".repeat(64), imageId, targetOrigin: undefined}),
    /incomplete/, "an unacknowledged publication must not be replaced")

    // Broker failure during the acknowledgement leaves the publication pending.
    chmodSync(origins, 0o755)
    assert.throws(() => controller().resolveRestore(container, firstDigest))
    chmodSync(origins, 0o700)
    assert.equal(restarted.read(container).phase, "published")
    // Retry succeeds; a repeated acknowledgement (crash before the kernel
    // recorded it) is a no-op. A foreign digest is still refused.
    controller().resolveRestore(container, firstDigest)
    controller().resolveRestore(container, firstDigest)
    assert.throws(() => controller().resolveRestore(container, "d".repeat(64)))
    assert.equal(restarted.read(container).phase, "resolved")

    // The next restore begins a fresh generation instead of rolling back.
    const secondDigest = "c".repeat(64)
    const second = restore(restarted, {oldContainerId: "first-container", oldHomeVolume: first.newHomeVolume,
      digest: secondDigest, containerId: "second-container"})
    assert.equal(second.oldHomeVolume, first.newHomeVolume)
    assert.equal(restarted.read(container).failedHomeVolume, undefined)
    controller().resolveRestore(container, secondDigest)
    assert.equal(restarted.read(container).phase, "resolved")
    assert.ok(restarted.read(container).retainedPreviousHomes.includes(first.oldHomeVolume))
    assert.throws(() => controller().resolveRestore(container, firstDigest), Error,
      "a stale acknowledgement cannot resolve a later publication")
  } finally {
    rmSync(root, {recursive: true, force: true})
  }
})
