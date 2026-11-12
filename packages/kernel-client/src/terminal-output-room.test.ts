import assert from "node:assert/strict"
import test from "node:test"
import { decodeTerminalOutput } from "./terminal-output-wire.js"

function wire(header: unknown, payload: Uint8Array) {
  const json = Buffer.from(JSON.stringify(header))
  const size = Buffer.alloc(4); size.writeUInt32BE(json.length)
  return Buffer.concat([Buffer.from("CXT1"), size, json, payload])
}

test("TR-11 Room terminal output restores raw bytes for the ordinary client", () => {
  const source = Buffer.from([0, 255, 27, 65])
  const result = decodeTerminalOutput(wire({ RoomTerminal: { result: { result: "output", output: { data_base64: [0, 4], next_offset: 9, gap: false } } } }, source))
  assert.deepEqual(result, { RoomTerminal: { result: { result: "output", output: { data_base64: source.toString("base64"), next_offset: 9, gap: false } } } })
})

test("TR-11 Room terminal replay restores ANSI and Unicode screen text", () => {
  const ansi = Buffer.from("\x1b[31mhello"), screen = Buffer.from("你好"), scrollback = Buffer.from("old🙂")
  const source = Buffer.concat([ansi, screen, scrollback])
  const result = decodeTerminalOutput(wire({ RoomTerminal: { result: { result: "snapshot", snapshot: { screen_ansi_base64: [0, ansi.length], screen_text: [ansi.length, screen.length], scrollback_text: [ansi.length + screen.length, scrollback.length], output_offset: 42 } } } }, source))
  assert.deepEqual(result, { RoomTerminal: { result: { result: "snapshot", snapshot: { screen_ansi_base64: ansi.toString("base64"), screen_text: "你好", scrollback_text: "old🙂", output_offset: 42 } } } })
})

test("TR-11 Room terminal raw replies reject gaps, overlap and trailing bytes", () => {
  for (const range of [[1, 2], [0, 1], [0, -1], [0, 4]]) {
    assert.throws(() => decodeTerminalOutput(wire({ RoomTerminal: { result: { result: "output", output: { data_base64: range } } } }, Buffer.from([1, 2]))))
  }
})
