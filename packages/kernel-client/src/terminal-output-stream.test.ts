import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import test from "node:test"
import * as browser from "./browser-relay-crypto.js"
import * as native from "./relay-crypto.js"
import { decodeTerminalOutput } from "./terminal-output-wire.js"

// The kernel test `mp08_room_terminal_stream_1006_snapshot_and_hash` proves its stream encoder
// writes exactly these headers, so each frame below carries what `Stream::send` puts on the wire.
const shapes = JSON.parse(readFileSync(
  new URL("../../../apps/kernel/src/local/api/tests/protocol_shapes/room-terminal-stream-1006.json", import.meta.url), "utf8",
)) as { output_header: { next_offset: number; bytes: [number, number] }; gap_header: Record<string, unknown> }

function frame(header: unknown, payload: Uint8Array) {
  const json = Buffer.from(JSON.stringify(header))
  const size = Buffer.alloc(4); size.writeUInt32BE(json.length)
  return Buffer.concat([Buffer.from("CXT1"), size, json, payload])
}

const page = Buffer.from("\u001b[31mred\u001b[0m café ✓\r\n".repeat(220)).subarray(0, shapes.output_header.next_offset)

test("a pushed Room terminal page from the kernel keeps its raw bytes", () => {
  assert.equal(page.length, shapes.output_header.next_offset)
  const result = decodeTerminalOutput(frame(shapes.output_header, page))
  assert.deepEqual(result, { ...shapes.output_header, bytes: new Uint8Array(page) })
})

test("a Room terminal gap page carries no bytes", () => {
  assert.deepEqual(decodeTerminalOutput(frame(shapes.gap_header, new Uint8Array())), { ...shapes.gap_header, bytes: new Uint8Array() })
})

test("a pushed Room terminal page refuses any range but the whole payload", () => {
  for (const bytes of [[1, 3], [0, 3], [0, 5], [0, -1], [0, 1.5], [0, 3, 0], null, "x"]) {
    assert.throws(() => decodeTerminalOutput(frame({ event: "room_terminal_output", next_offset: 9, bytes }, Buffer.from([1, 2, 3, 4]))))
  }
})

test("a pushed Room terminal page crosses the browser and Node relay decrypt paths", async () => {
  const wire = frame(shapes.output_header, page)
  const recipient = await browser.createRelayKeypair()
  const encrypted = native.encryptRelayPayload(recipient.publicKeyBase64, wire)
  assert.deepEqual(
    await browser.decryptRelayEvent(recipient.privateKey, encrypted.payload, encrypted.payload.sender_public_key),
    { ...shapes.output_header, bytes: new Uint8Array(page) },
  )
  const nativeRecipient = native.createRelayKeypair()
  const nativeEncrypted = native.encryptRelayPayload(nativeRecipient.publicKeyBase64, wire)
  assert.deepEqual(
    JSON.parse(native.decryptRelayPayload(nativeRecipient.privateKey, nativeEncrypted.payload)),
    { ...shapes.output_header, bytes: Array.from(page) },
  )
})
