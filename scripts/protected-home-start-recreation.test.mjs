// MP-03/MP-08/MP-10/MP-11: normal Start after saved-home destruction.
import test from "node:test"
import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { chmodSync, mkdirSync, mkdtempSync, rmSync } from "node:fs"
import { join } from "node:path"
import { createManagedLayoutController } from "../apps/kernel/slice-linux-docker/protected-managed-layout.mjs"
import { createHomeGenerationStore } from "../apps/kernel/slice-linux-docker/protected-home-generation.mjs"
import { recordManagedImageProof } from "../apps/kernel/slice-linux-docker/protected-image-proof.mjs"
import { readProtectedLayoutReceipt, writeProtectedLayoutReceipt } from "../apps/kernel/slice-linux-docker/protected-layout-store.mjs"

const container = "chariox-slice-start-recreation"
const imageId = `sha256:${"a".repeat(64)}`
const sourceDigest = `sha256:${"b".repeat(64)}`
const archiveDigest = "c".repeat(64)

function fixture(t) {
  const root = mkdtempSync(join(process.env.CHARIOX_HOME ?? process.env.HOME, ".chariox-start-recreation-test-"))
  t.after(() => rmSync(root, {recursive: true, force: true}))
  chmodSync(root, 0o711)
  for (const directory of ["receipts", "images", "capture-origins"]) mkdirSync(join(root, directory), {mode: 0o700})
  const layout = {version: 1, sliceId: container, containerId: "captured-container", imageId,
    homeVolume: `${container}-home`}
  writeProtectedLayoutReceipt(join(root, "receipts"), container, layout)
  recordManagedImageProof(join(root, "images"), sourceDigest,
    {Id: imageId, Config: {User: "slice"}, RootFS: {Layers: [imageId]}}, "d".repeat(64))
  const originKey = createHash("sha256").update(`${layout.containerId}\0${archiveDigest}`).digest("hex")
  const origin = {version: 1, sliceId: originKey, container, containerId: layout.containerId,
    homeVolume: layout.homeVolume, digest: archiveDigest}
  writeProtectedLayoutReceipt(join(root, "capture-origins"), originKey, origin)
  const inventory = {volumes: [], containers: [], volumeStatus: 0, containerStatus: 0, image: imageId}
  const controller = createManagedLayoutController({root, sourceDigest, dataOwner: process.getuid(), docker: args => {
    if (args.join(" ") === "volume ls --format {{.Name}}") {
      return {status: inventory.volumeStatus, stdout: inventory.volumes.join("\n")}
    }
    if (args.join(" ") === "ps -a --format {{.Names}}") {
      return {status: inventory.containerStatus, stdout: inventory.containers.join("\n")}
    }
    if (args[0] === "image" && args[1] === "inspect") {
      return {status: 0, stdout: JSON.stringify([{Id: inventory.image}])}
    }
    if (args[0] === "container" && args[1] === "inspect") {
      return {status: 0, stdout: JSON.stringify([{Id: layout.containerId}])}
    }
    throw new Error(`unexpected Docker operation: ${args.join(" ")}`)
  }})
  // The retained-identity seam is independently covered by protected-slice-layout
  // tests. This public-metadata fixture creates no runtime identities or keys.
  controller.homeVolume = () => readProtectedLayoutReceipt(join(root, "receipts"), container).homeVolume
  const environment = () => ({CHARIOX_SLICE_NAME: container, CHARIOX_SLICE_PRIVATE_HOST_ROOT: join(root, "private"),
    CHARIOX_SLICE_SAVED_HOME_ARCHIVE: "/proc/123/fd/4", CHARIOX_SLICE_DOCKER_IMAGE: imageId})
  return {root, layout, origin, inventory, controller, environment, store: createHomeGenerationStore(root)}
}

test("MP-08/MP-10: Start recreates an absent retained home with a fresh protected generation", t => {
  const f = fixture(t)
  const env = f.environment()
  f.controller.beginRestore(env, archiveDigest, "provision")
  assert.match(env.CHARIOX_SLICE_RESTORE_GENERATION ?? "", /^[a-f0-9]{32}$/,
    "normal Start must provide the generation required by protected-home-restore")
  assert.equal(env.CHARIOX_SLICE_HOME_VOLUME, `${container}-home-g${env.CHARIOX_SLICE_RESTORE_GENERATION}`)
  assert.equal(env.CHARIOX_SLICE_RESTORE_DIGEST, archiveDigest)
  assert.equal(env.CHARIOX_SLICE_PREVIOUS_HOME_VOLUME, f.layout.homeVolume)
  const pending = f.store.read(container)
  assert.equal(pending.phase, "preparing")
  assert.equal(pending.oldContainerId, undefined, "absent container recreation resolves as initialization, without a backup transaction")
  assert.deepEqual(pending.targetOrigin, f.origin)
  assert.deepEqual(readProtectedLayoutReceipt(join(f.root, "receipts"), container), f.layout,
    "preparation must preserve the retained receipt")
  const parameters = {container, token: pending.token, volume: pending.newHomeVolume, digest: archiveDigest}
  assert.throws(() => f.store.requireReady(parameters), /incomplete/)
  f.store.complete(parameters)
  const published = {...f.layout, containerId: "recreated-container", homeVolume: pending.newHomeVolume}
  writeProtectedLayoutReceipt(join(f.root, "receipts"), container, published)
  f.store.publish(parameters)
  // Simulate a broker restart between layout publication and journal resolution.
  f.inventory.volumes = [pending.newHomeVolume]
  f.controller.beginRestore(f.environment(), archiveDigest, "provision")
  assert.equal(f.store.read(container).phase, "resolved")
  // A named-backup restore after ordinary Start must begin a new transaction.
  f.controller.beginRestore(f.environment(), archiveDigest, "restore-state")
  const backup = f.store.read(container)
  assert.equal(backup.oldContainerId, published.containerId)
  assert.equal(backup.oldHomeVolume, pending.newHomeVolume)
  assert.notEqual(backup.token, pending.token)
})

test("MP-03/MP-08: Start preserves an existing retained home, including unsaved changes", t => {
  const f = fixture(t)
  f.inventory.volumes = [f.layout.homeVolume]
  const env = f.environment()
  f.controller.beginRestore(env, archiveDigest, "provision")
  assert.equal(env.CHARIOX_SLICE_HOME_VOLUME, f.layout.homeVolume)
  assert.equal(env.CHARIOX_SLICE_RESTORE_GENERATION, undefined)
  assert.equal(f.store.read(container), null)
})

for (const failure of ["volume inventory", "container inventory", "container remains", "unproven image", "foreign capture"]) {
  test(`MP-03/MP-10: Start refuses ${failure} before generation preparation`, t => {
    const f = fixture(t)
    if (failure === "volume inventory") f.inventory.volumeStatus = 1
    if (failure === "container inventory") f.inventory.containerStatus = 1
    if (failure === "container remains") f.inventory.containers = [container]
    if (failure === "unproven image") f.inventory.image = `sha256:${"e".repeat(64)}`
    const digest = failure === "foreign capture" ? "f".repeat(64) : archiveDigest
    assert.throws(() => f.controller.beginRestore(f.environment(), digest, "provision"))
    assert.equal(f.store.read(container), null)
    assert.deepEqual(readProtectedLayoutReceipt(join(f.root, "receipts"), container), f.layout)
  })
}

test("MP-03/MP-10: interrupted Start recreation cannot silently reuse an incomplete home", t => {
  const f = fixture(t)
  const env = f.environment()
  f.controller.beginRestore(env, archiveDigest, "provision")
  assert.equal(f.store.read(container)?.phase, "preparing")
  f.inventory.volumes = [env.CHARIOX_SLICE_HOME_VOLUME]
  assert.throws(() => f.controller.beginRestore(f.environment(), archiveDigest, "provision"), /incomplete/)
  assert.equal(f.store.read(container).phase, "preparing")
  assert.deepEqual(readProtectedLayoutReceipt(join(f.root, "receipts"), container), f.layout)
})
