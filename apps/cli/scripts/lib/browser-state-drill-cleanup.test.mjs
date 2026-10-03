import assert from "node:assert/strict"
import test from "node:test"

import { browserStateCleanupFailure, cleanupBrowserStateImages } from "./browser-state-drill-cleanup.mjs"

test("browser state cleanup accepts a fully released drill", () => {
  assert.equal(browserStateCleanupFailure({
    dockerAvailable: true,
    containerGone: true,
    fixtureSidecarGone: true,
    volumeGone: true,
    savedImageGone: true,
    backupImagesGone: true,
    fixtureWorkspaceRemoved: true,
    tempRootRemoved: true,
    listenersReleased: true,
    occupiedPorts: [],
  }), null)
})

test("browser state cleanup names every leaked resource", () => {
  const failure = browserStateCleanupFailure({
    dockerAvailable: false,
    containerGone: false,
    fixtureSidecarGone: false,
    volumeGone: false,
    savedImageGone: false,
    backupImagesGone: false,
    fixtureWorkspaceRemoved: false,
    tempRootRemoved: false,
    listenersReleased: false,
    occupiedPorts: [55100, 55101],
  })

  assert.match(failure?.message ?? "", /container/)
  assert.match(failure?.message ?? "", /fixture sidecar/)
  assert.match(failure?.message ?? "", /Docker verification/)
  assert.match(failure?.message ?? "", /volume/)
  assert.match(failure?.message ?? "", /saved image/)
  assert.match(failure?.message ?? "", /backup images/)
  assert.match(failure?.message ?? "", /fixture workspace/)
  assert.match(failure?.message ?? "", /runtime root/)
  assert.match(failure?.message ?? "", /ports 55100, 55101/)
})


test("rollback cleanup removes only the exact drill owner using its inspected immutable image ID", async () => {
  const slice = { id: "slice-1", owner_kernel_id: "kernel-drill", owner_machine_id: "machine-drill" }
  const ownId = "sha256:" + "a".repeat(64)
  const labels = {
    "io.chariox.slice.id": slice.id,
    "io.chariox.slice.owner-kernel-id": slice.owner_kernel_id,
    "io.chariox.slice.owner-machine-id": slice.owner_machine_id,
  }
  const images = new Map([
    ["own", { Id: ownId, Config: { Labels: labels } }],
    ["foreign", { Id: "sha256:" + "b".repeat(64), Config: { Labels: { ...labels, "io.chariox.slice.owner-kernel-id": "other-kernel" } } }],
    ["unlabelled", { Id: "sha256:" + "c".repeat(64), Config: { Labels: null } }],
    ["incomplete", { Id: "sha256:" + "d".repeat(64), Config: { Labels: { "io.chariox.slice.id": slice.id } } }],
  ])
  const inspected = []
  const removed = []
  const result = await cleanupBrowserStateImages({
    imageRefs: ["own", "foreign", "unlabelled", "incomplete", "preexisting"],
    imagesBefore: new Set(["preexisting"]), slice,
    inspectImage: async ref => { inspected.push(ref); return images.get(ref) ?? null },
    removeImage: async ref => { removed.push(ref) },
  })
  assert.deepEqual(removed, [ownId])
  assert.deepEqual(inspected, ["own", ownId, "foreign", "unlabelled", "incomplete"])
  assert.deepEqual(result, {
    removedImageRefs: ["own"],
    preservedImageRefs: ["foreign", "unlabelled", "incomplete"],
    unresolvedImageRefs: [],
  })
})


const ownSlice = { id: "slice-1", owner_kernel_id: "kernel-drill", owner_machine_id: "machine-drill" }
const ownImageId = "sha256:" + "e".repeat(64)
const ownLabels = {
  "io.chariox.slice.id": ownSlice.id,
  "io.chariox.slice.owner-kernel-id": ownSlice.owner_kernel_id,
  "io.chariox.slice.owner-machine-id": ownSlice.owner_machine_id,
}
const ownImage = { Id: ownImageId, Config: { Labels: ownLabels } }

for (const [key, value] of [
  ["io.chariox.slice.id", "slice-other"],
  ["io.chariox.slice.owner-machine-id", "machine-other"],
]) test(`fallback cleanup preserves a foreign ${key}`, async () => {
  const removed = []
  const result = await cleanupBrowserStateImages({
    imageRefs: ["foreign"], imagesBefore: new Set(), slice: ownSlice,
    inspectImage: async () => ({ ...ownImage, Config: { Labels: { ...ownLabels, [key]: value } } }),
    removeImage: async ref => removed.push(ref),
  })
  assert.deepEqual(removed, [])
  assert.deepEqual(result.preservedImageRefs, ["foreign"])
  assert.deepEqual(result.unresolvedImageRefs, [])
})

test("fallback cleanup never deletes without a complete drill identity", async () => {
  for (const slice of [null, { ...ownSlice, owner_kernel_id: "" }, { ...ownSlice, owner_machine_id: null }]) {
    const result = await cleanupBrowserStateImages({
      imageRefs: ["candidate"], imagesBefore: new Set(), slice,
      inspectImage: async () => assert.fail("missing drill ownership must not inspect or delete candidates"),
      removeImage: async () => assert.fail("missing drill ownership must not delete candidates"),
    })
    assert.deepEqual(result.unresolvedImageRefs, ["candidate"])
    assert.deepEqual(result.removedImageRefs, [])
  }
})

test("fallback cleanup preserves ambiguous label values", async () => {
  const result = await cleanupBrowserStateImages({
    imageRefs: ["ambiguous"], imagesBefore: new Set(), slice: ownSlice,
    inspectImage: async () => ({ ...ownImage, Config: { Labels: { ...ownLabels, "io.chariox.slice.id": [ownSlice.id] } } }),
    removeImage: async () => assert.fail("ambiguous ownership must not delete candidates"),
  })
  assert.deepEqual(result.preservedImageRefs, ["ambiguous"])
  assert.deepEqual(result.unresolvedImageRefs, [])
})

for (const mode of ["remove-failure", "still-present", "verification-failure"]) {
  test(`owned fallback cleanup reports ${mode}`, async () => {
    const removed = []
    const result = await cleanupBrowserStateImages({
      imageRefs: ["owned"], imagesBefore: new Set(), slice: ownSlice,
      inspectImage: async ref => {
        if (ref === ownImageId && mode === "verification-failure") throw new Error("synthetic inspection failure")
        return ownImage
      },
      removeImage: async ref => {
        removed.push(ref)
        if (mode === "remove-failure") throw new Error("synthetic removal failure")
      },
    })
    assert.deepEqual(removed, [ownImageId])
    assert.deepEqual(result.unresolvedImageRefs, ["owned"])
    assert.deepEqual(result.removedImageRefs, [])
    assert.match(browserStateCleanupFailure({
      dockerAvailable: true, containerGone: true, volumeGone: true, savedImageGone: true,
      backupImagesGone: false, tempRootRemoved: true, listenersReleased: true,
      rollbackImagesUnresolved: result.unresolvedImageRefs,
    }).message, /unresolved rollback images owned/)
  })
}

test("owned fallback images require an immutable ID and inspected candidates cannot fail silently", async () => {
  for (const image of [{ ...ownImage, Id: "movable-tag" }, new Error("synthetic inspect failure")]) {
    const result = await cleanupBrowserStateImages({
      imageRefs: ["owned"], imagesBefore: new Set(), slice: ownSlice,
      inspectImage: async () => { if (image instanceof Error) throw image; return image },
      removeImage: async () => assert.fail("unresolved image identity must not delete candidates"),
    })
    assert.deepEqual(result.unresolvedImageRefs, ["owned"])
  }
})

test("foreign fallback images do not fail cleanup and failed inventories are explicit", () => {
  const clean = {
    dockerAvailable: true, containerGone: true, volumeGone: true, savedImageGone: true,
    backupImagesGone: true, tempRootRemoved: true, listenersReleased: true,
    rollbackImagesPreserved: ["foreign"],
  }
  assert.equal(browserStateCleanupFailure(clean), null)
  assert.match(browserStateCleanupFailure({ ...clean, rollbackImageInventoryFailed: true }).message, /rollback image inventory unavailable/)
  assert.match(browserStateCleanupFailure({ ...clean, stateImageInventoryFailed: true }).message, /state image inventory unavailable/)
})
