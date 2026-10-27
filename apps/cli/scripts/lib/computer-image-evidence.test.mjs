// MP-08/MP-10/MP-11: synthetic receipts never establish a model/viewer gate.
import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import test from "node:test"
import { assertRoomComputerImage, assertRoomComputerSurface } from "./computer-image-evidence.mjs"

const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aWQAAAABJRU5ErkJggg==", "base64")
const expected = { sessionId: "room", sliceId: "surface", agentId: "reader",
  viewport: { desktop_pixel_width: 1, desktop_pixel_height: 1 } }
function fixture() {
  return { content: [{ type: "image", mimeType: "image/png", data: png.toString("base64") },
    { type: "text", text: '{"artifact_id":"opaque-image"}' }], structuredContent: {
    source: "computer_controller", session_id: "room", slice_id: "surface", agent_id: "reader",
    artifact_id: "opaque-image", mime_type: "image/png", size_bytes: png.length,
    sha256: createHash("sha256").update(png).digest("hex") } }
}

test("native MCP image retains exact bytes and canonical dimensions", () => {
  assert.deepEqual(assertRoomComputerImage(fixture(), expected).bytes, png)
})

for (const [label, mutate] of [
  ["artifact-only result", r => { r.content.shift() }],
  ["duplicate image", r => { r.content.push(r.content[0]) }],
  ["foreign Room", r => { r.structuredContent.session_id = "foreign" }],
  ["foreign surface", r => { r.structuredContent.slice_id = "foreign" }],
  ["foreign agent", r => { r.structuredContent.agent_id = "foreign" }],
  ["missing bytes", r => { r.content[0].data = "" }],
  ["noncanonical bytes", r => { r.content[0].data += "!" }],
  ["byte-size mismatch", r => { r.structuredContent.size_bytes++ }],
  ["byte-digest mismatch", r => { r.structuredContent.sha256 = "0".repeat(64) }],
  ["MIME mismatch", r => { r.content[0].mimeType = "text/plain" }],
  ["structured image duplication", r => { r.structuredContent.image_base64 = r.content[0].data }],
  ["text image duplication", r => { r.content[1].text = r.content[0].data }],
]) {
  test(`rejects ${label}`, () => {
    const reply = fixture()
    mutate(reply)
    assert.throws(() => assertRoomComputerImage(reply, expected))
  })
}

test("canonical geometry comes from the kernel, not artifact metadata", () => {
  assert.throws(() => assertRoomComputerImage(fixture(), { ...expected,
    viewport: { desktop_pixel_width: 1280, desktop_pixel_height: 800 } }))
})

test("one kernel binding joins input and image authority and rejects stale generations", () => {
  const receipt = { environment: { environment_id: "env", session_id: "room", runtime_generation: 4, viewport: expected.viewport },
    binding: { session_id: "room", slice_id: "surface" },
    action: { action_id: "action", actor_id: "agent:reader", session_id: "room", environment_id: "env", runtime_generation: 4 },
    status: { session_id: "room", slice_id: "surface", agent_id: "reader", canonical_viewport: expected.viewport },
    screenshot: fixture() }
  assert.deepEqual(assertRoomComputerSurface(receipt, "reader").bytes, png)
  receipt.action.runtime_generation = 3
  assert.throws(() => assertRoomComputerSurface(receipt, "reader"))
  receipt.action.runtime_generation = 4
  receipt.action.environment_id = "another-authority"
  assert.throws(() => assertRoomComputerSurface(receipt, "reader"))
})
