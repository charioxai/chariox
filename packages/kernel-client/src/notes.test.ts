import assert from "node:assert/strict"
import test from "node:test"
import { notesRequest, noteOverlayBox, notesMinimumProtocolVersion } from "./notes.js"

test("MD-N3 / MP-08: reject old/unknown kernels before notes requests", () => {
  for (const version of [417, 423, NaN, Infinity, 443.5]) assert.throws(() => notesRequest({ op: "capture_selection", window: { kind: "panel", window_id: "x" } }, version))
  assert.equal(notesMinimumProtocolVersion, 443)
  assert.deepEqual(notesRequest({ op: "capture_selection", window: { kind: "panel", window_id: "transcript" } }, 443), { Notes: { command: { op: "capture_selection", window: { kind: "panel", window_id: "transcript" } } } })
})
test("MD-N2 / MP-08: overlay maps page coordinates, clips and rejects invalid geometry", () => {
  assert.deepEqual(noteOverlayBox({ x: -10, y: 40, width: 30, height: 20 }, { width: 1280, height: 800 }, { x: 200, y: 100, width: 640, height: 400 }), { x: 200, y: 120, width: 10, height: 10 })
  assert.equal(noteOverlayBox({ x: 0, y: 0, width: NaN, height: 1 }, { width: 1, height: 1 }, { x: 0, y: 0, width: 1, height: 1 }), null)
})
