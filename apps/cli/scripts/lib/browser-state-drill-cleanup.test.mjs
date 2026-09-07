import assert from "node:assert/strict"
import test from "node:test"

import { browserStateCleanupFailure } from "./browser-state-drill-cleanup.mjs"

test("browser state cleanup accepts a fully released drill", () => {
  assert.equal(browserStateCleanupFailure({
    dockerAvailable: true,
    containerGone: true,
    volumeGone: true,
    savedImageGone: true,
    backupImagesGone: true,
    tempRootRemoved: true,
    listenersReleased: true,
    occupiedPorts: [],
  }), null)
})

test("browser state cleanup names every leaked resource", () => {
  const failure = browserStateCleanupFailure({
    dockerAvailable: false,
    containerGone: false,
    volumeGone: false,
    savedImageGone: false,
    backupImagesGone: false,
    tempRootRemoved: false,
    listenersReleased: false,
    occupiedPorts: [55100, 55101],
  })

  assert.match(failure?.message ?? "", /container/)
  assert.match(failure?.message ?? "", /Docker verification/)
  assert.match(failure?.message ?? "", /volume/)
  assert.match(failure?.message ?? "", /saved image/)
  assert.match(failure?.message ?? "", /backup images/)
  assert.match(failure?.message ?? "", /runtime root/)
  assert.match(failure?.message ?? "", /ports 55100, 55101/)
})
