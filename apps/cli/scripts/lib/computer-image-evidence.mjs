// MP-08/MP-10/MP-11: transport evidence only. An official provider's pixel-only
// acknowledgement and actual Web/TUI observations remain separate live gates.
import assert from "node:assert/strict"
import { createHash } from "node:crypto"

const MAX_IMAGE_BYTES = 16 * 1024 * 1024
const PNG_SIGNATURE = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])

export function assertRoomComputerImage(reply, { sessionId, sliceId, agentId, viewport }) {
  assert.equal(reply.isError ?? false, false, "Computer screenshot MCP call failed")
  const metadata = reply.structuredContent
  assert.equal(metadata?.source, "computer_controller", "Computer screenshot source differs")
  assert.equal(metadata.session_id, sessionId, "Computer screenshot Room differs")
  assert.equal(metadata.slice_id, sliceId, "Computer screenshot surface differs")
  assert.equal(metadata.agent_id, agentId, "Computer screenshot agent differs")
  assert.ok(metadata.artifact_id, "Computer screenshot artifact identity missing")
  assert.equal(metadata.mime_type, "image/png", "Computer screenshot MIME differs")
  assert.equal(metadata.image_base64, undefined, "image bytes duplicated into structured content")
  const images = reply.content?.filter(part => part.type === "image") ?? []
  assert.equal(images.length, 1, "Computer screenshot needs exactly one native MCP image; an artifact reference is insufficient")
  const image = images[0]
  assert.equal(image.mimeType, "image/png", "native image MIME differs")
  assert.ok(typeof image.data === "string" && image.data.length <= 4 * Math.ceil(MAX_IMAGE_BYTES / 3),
    "native image missing or exceeds byte limit")
  const bytes = Buffer.from(image.data, "base64")
  assert.equal(bytes.toString("base64"), image.data, "native image Base64 is not canonical")
  assert.ok(bytes.length >= 33 && bytes.length <= MAX_IMAGE_BYTES, "native PNG size outside supported bounds")
  assert.equal(bytes.length, metadata.size_bytes, "native image size differs from artifact")
  assert.equal(createHash("sha256").update(bytes).digest("hex"), metadata.sha256, "native image digest differs from artifact")
  assert.ok(bytes.subarray(0, 8).equals(PNG_SIGNATURE), "native image PNG signature missing")
  assert.equal(bytes.readUInt32BE(8), 13, "native image IHDR length differs")
  assert.equal(bytes.toString("ascii", 12, 16), "IHDR", "native image PNG header missing")
  assert.equal(bytes.readUInt32BE(16), viewport.desktop_pixel_width, "native image width differs from canonical geometry")
  assert.equal(bytes.readUInt32BE(20), viewport.desktop_pixel_height, "native image height differs from canonical geometry")
  for (const part of reply.content.filter(part => part.type === "text")) {
    assert.ok(!part.text.includes(image.data), "native image duplicated into text context")
  }
  return { artifactId: metadata.artifact_id, sha256: metadata.sha256, sizeBytes: bytes.length,
    width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20), bytes }
}

export function assertRoomComputerSurface({ environment, binding, action, status, screenshot }, agentId) {
  assert.ok(environment.environment_id, "kernel Environment identity missing")
  assert.equal(binding.session_id, environment.session_id, "surface binding belongs to another Room")
  assert.equal(action.session_id, environment.session_id, "Computer input belongs to another Room")
  assert.equal(action.environment_id, environment.environment_id, "Computer input has another authority")
  assert.equal(action.runtime_generation, environment.runtime_generation, "Computer input uses stale runtime generation")
  assert.equal(action.actor_id, `agent:${agentId}`, "Computer input actor differs")
  assert.ok(action.action_id, "Computer input action identity missing")
  for (const observation of [status, screenshot.structuredContent]) {
    assert.equal(observation.session_id, environment.session_id, "observation Room differs")
    assert.equal(observation.slice_id, binding.slice_id, "input and observation surfaces differ")
    assert.equal(observation.agent_id, agentId, "observation agent differs")
  }
  assert.deepEqual(status.canonical_viewport, environment.viewport, "status and kernel canonical geometry differ")
  return assertRoomComputerImage(screenshot, { sessionId: environment.session_id,
    sliceId: binding.slice_id, agentId, viewport: environment.viewport })
}
