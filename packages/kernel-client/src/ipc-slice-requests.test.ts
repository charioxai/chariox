import assert from "node:assert/strict"
import test from "node:test"
import { createSliceRequest } from "./ipc-slice-requests.js"

test("slice creation forwards an explicit display backend on the shared client path", () => {
  const request = createSliceRequest({ name: "desktop", displayMode: "headed", displayBackend: "selkies" })
  assert.equal(request.CreateSlice.display_backend, "selkies")
  assert.equal(request.CreateSlice.display_mode, "headed")
})

test("legacy slice creation does not add a backend field", () => {
  assert.equal(Object.hasOwn(createSliceRequest({ name: "legacy" }).CreateSlice, "display_backend"), false)
})
