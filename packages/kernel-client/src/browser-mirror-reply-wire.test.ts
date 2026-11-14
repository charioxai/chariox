import assert from "node:assert/strict"
import test from "node:test"
import { decodeTerminalOutput } from "./terminal-output-wire.js"

function frame(header: unknown, payload: Uint8Array) {
  const json = Buffer.from(JSON.stringify(header))
  const size = Buffer.alloc(4); size.writeUInt32BE(json.length)
  return Buffer.concat([Buffer.from("CXT1"), size, json, payload])
}

const body = Buffer.from([1, 2, 3, 250, 251])
const font = Buffer.from([9, 8, 7])

test("a mirror reply gets its deflated body and whole resources back as base64 text", () => {
  const header = { KernelBrowser: { result: { wire: 2, sequence: 3, packet_base64: [0, 5], resources: [{ key: "f", data_base64: [5, 3] }, { key: "s", data_base64: "QUJD" }] } } }
  assert.deepEqual(decodeTerminalOutput(frame(header, Buffer.concat([body, font]))), {
    KernelBrowser: { result: { wire: 2, sequence: 3, packet_base64: body.toString("base64"), resources: [{ key: "f", data_base64: font.toString("base64") }, { key: "s", data_base64: "QUJD" }] } },
  })
})

test("a mirror reply refuses a range that leaves its payload", () => {
  for (const [range, payload] of [[[0, 9], body], [[1, 4], body], [[0, 4], body]] as const) {
    assert.throws(() => decodeTerminalOutput(frame({ KernelBrowser: { result: { packet_base64: range } } }, payload)))
  }
})
