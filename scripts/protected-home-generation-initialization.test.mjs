import test from "node:test"
import assert from "node:assert/strict"
import { mkdtempSync, chmodSync, mkdirSync, rmSync } from "node:fs"
import { join } from "node:path"
import { createHomeGenerationStore } from "../apps/kernel/slice-linux-docker/protected-home-generation.mjs"
import { readProtectedLayoutReceipt, writeProtectedLayoutReceipt } from "../apps/kernel/slice-linux-docker/protected-layout-store.mjs"

const container = "chariox-slice-new-from-state"
const imageId = `sha256:${"a".repeat(64)}`
const archiveDigest = "b".repeat(64)
function fixture(run) {
  const root = mkdtempSync(join(process.env.HOME, ".chariox-initial-generation-test-"))
  chmodSync(root, 0o711)
  try { run(root, createHomeGenerationStore(root)) }
  finally { rmSync(root, {recursive: true}) }
}
function initialize(store) {
  const record = store.begin({container, oldHomeVolume: `${container}-home`, archiveDigest, imageId,
    targetOrigin: {container: "chariox-slice-source", containerId: "source-container", digest: archiveDigest}})
  const parameters = {container, token: record.token, volume: record.newHomeVolume, digest: archiveDigest}
  const layout = {version: 1, sliceId: container, containerId: "new-container", imageId, homeVolume: record.newHomeVolume}
  return {record, parameters, layout}
}

for (const interruptedBeforeJournalPublication of [false, true]) {
  test(`saved-state initialization resolves after durable layout, journal interrupted=${interruptedBeforeJournalPublication}`, () => fixture((root, store) => {
    const {record, parameters, layout} = initialize(store)
    store.complete(parameters)
    const receipts = join(root, "receipts")
    mkdirSync(receipts, {mode: 0o700})
    writeProtectedLayoutReceipt(receipts, container, layout)
    if (!interruptedBeforeJournalPublication) store.publish(parameters)
    const restarted = createHomeGenerationStore(root)
    restarted.resolveInitialization(container, readProtectedLayoutReceipt(receipts, container))
    assert.equal(restarted.read(container).phase, "resolved")
    assert.equal(restarted.read(container).oldHomeVolume, record.oldHomeVolume)
    const backupDigest = "c".repeat(64)
    const restore = restarted.begin({container, oldHomeVolume: layout.homeVolume,
      oldContainerId: layout.containerId, archiveDigest: backupDigest, imageId,
      targetOrigin: {container, containerId: layout.containerId, homeVolume: layout.homeVolume, digest: backupDigest}})
    const next = {container, token: restore.token, volume: restore.newHomeVolume, digest: backupDigest}
    restarted.complete(next); restarted.publish(next)
    const restoredLayout = {...layout, containerId: "restored-container", homeVolume: restore.newHomeVolume}
    restarted.resolveInitialization(container, restoredLayout)
    assert.equal(restarted.read(container).phase, "published", "replacement still requires kernel publication acknowledgement")
    restarted.resolve(container, restoredLayout.homeVolume)
    assert.equal(restarted.read(container).phase, "resolved")
    assert.ok(restarted.read(container).retainedPreviousHomes.includes(record.oldHomeVolume))
  }))
}

test("initialization cannot resolve incomplete extraction or a foreign published layout", () => fixture((_root, store) => {
  const {parameters, layout} = initialize(store)
  assert.throws(() => store.resolveInitialization(container, layout))
  assert.equal(store.read(container).phase, "preparing")
  store.complete(parameters)
  for (const foreign of [undefined, {...layout, sliceId: "foreign"}, {...layout, containerId: ""},
    {...layout, homeVolume: "foreign-home"}, {...layout, imageId: `sha256:${"d".repeat(64)}`}]) {
    assert.throws(() => store.resolveInitialization(container, foreign))
    assert.equal(store.read(container).phase, "ready")
  }
  store.publish(parameters)
  assert.throws(() => store.resolveInitialization(container, {...layout, homeVolume: "foreign-home"}))
  assert.equal(store.read(container).phase, "published")
  store.resolveInitialization(container, layout)
  store.resolveInitialization(container, layout)
  assert.equal(store.read(container).phase, "resolved")
}))
