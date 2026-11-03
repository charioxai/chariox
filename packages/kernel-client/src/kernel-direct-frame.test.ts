import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import test from "node:test"
import { expandDirectFrame } from "./kernel-direct-frame.js"

const fixture = JSON.parse(readFileSync(new URL("../../../apps/kernel/src/local/api/tests/protocol_shapes/direct-compact-961.json", import.meta.url), "utf8")) as {
  frames: { kind: string; id: string; nonce: string; event_id?: number; wire_hex: string }[]
}
const decode = (bytes: Uint8Array) => JSON.parse(expandDirectFrame(Uint8Array.from(bytes).buffer, "pinned-key"))

test("MP-08/MP-09/MP-10/MP-11 binary routing matches kernel reply and event snapshots", () => {
  for (const frame of fixture.frames) {
    const bytes = Buffer.from(frame.wire_hex, "hex"), result = decode(bytes)
    assert.equal(result.kind, frame.kind)
    assert.equal(result.request_id ?? result.subscription_id, frame.id)
    if (frame.event_id !== undefined) assert.equal(result.event_id, frame.event_id)
    const payload = result.encrypted_response ?? result.encrypted_event
    assert.equal(payload.sender_public_key, "pinned-key")
    assert.equal(payload.nonce, frame.nonce)
    assert.equal(payload.ciphertext, "AAAAAAAAAAAAAAAAAAAAAA==")
    assert.equal(bytes.length, (frame.event_id === undefined ? 35 : 43) + Buffer.byteLength(frame.id))
  }
})

test("MP-11 binary routing rejects invalid kinds, bounds, UTF-8, sequence and old headers", () => {
  const reply = Buffer.from(fixture.frames[0]!.wire_hex, "hex")
  for (const frame of fixture.frames) {
    const bytes = Buffer.from(frame.wire_hex, "hex")
    for (let size = 0; size < bytes.length; size++) assert.throws(() => decode(bytes.subarray(0, size)))
  }
  const invalid: Buffer[] = []
  const change = (offset: number, value: number) => { const bytes = Buffer.from(reply); bytes[offset] = value; invalid.push(bytes) }
  change(3, 49) // CXL1 is retired.
  change(4, 2)
  change(6, 0)
  change(5, 5)
  change(19, 0xff)
  const event = Buffer.from(fixture.frames[1]!.wire_hex, "hex")
  event.writeBigUInt64BE(1n << 53n, 19); invalid.push(event)
  const legacy = Buffer.from('{"r":"r","n":"AAAAAAAAAAAAAAAA","sender_public_key":"other"}')
  const old = Buffer.concat([Buffer.from("CXL1"), Buffer.alloc(4), legacy, Buffer.alloc(16)])
  old.writeUInt32BE(legacy.length, 4); invalid.push(old)
  for (const bytes of invalid) assert.throws(() => decode(bytes))
})

test("MP-08/MP-11 routing preserves exact UTF-8 ids and enforces their byte limit", () => {
  for (const id of ["\ufeffroute", "é".repeat(512), "é".repeat(513)]) {
    const encoded = Buffer.from(id), bytes = Buffer.concat([Buffer.from("CXL2"), Buffer.alloc(15), encoded, Buffer.alloc(16)])
    bytes.writeUInt16BE(encoded.length, 5)
    if (encoded.length > 1024) assert.throws(() => decode(bytes))
    else assert.equal(decode(bytes).request_id, id)
  }
})
