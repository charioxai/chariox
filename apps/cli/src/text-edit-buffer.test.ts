import assert from "node:assert/strict"
import test from "node:test"

import { applyEditKey, bufferAtEnd, cleanPastedText, typedText, type TextBuffer } from "./text-edit-buffer.js"

const press = (buffer: TextBuffer, name: string, extra: Record<string, unknown> = {}, multiline = true) =>
  applyEditKey(buffer, { name, ...extra }, multiline)

test("typing inserts at the cursor and backspace removes one character", () => {
  let buffer: TextBuffer = bufferAtEnd("ac")
  buffer = press(buffer, "left")!
  buffer = press(buffer, "b", { sequence: "b" })!
  assert.deepEqual(buffer, { text: "abc", cursor: 2 })
  buffer = press(buffer, "backspace")!
  assert.deepEqual(buffer, { text: "ac", cursor: 1 })
  buffer = press(buffer, "delete")!
  assert.deepEqual(buffer, { text: "a", cursor: 1 })
})

test("shifted keys without a sequence type the upper case letter and space types a space", () => {
  assert.equal(typedText({ name: "a", shift: true }), "A")
  assert.equal(typedText({ name: "space" }), " ")
  assert.equal(typedText({ name: "f6" }), "")
  assert.equal(typedText({ name: "c", ctrl: true }), "")
  assert.equal(typedText({ name: "return", sequence: "\r" }), "")
})

test("backspace removes a whole emoji", () => {
  assert.deepEqual(press(bufferAtEnd("a😀"), "backspace"), { text: "a", cursor: 1 })
})

test("multiline buffers take newlines and move between lines keeping the column", () => {
  let buffer = bufferAtEnd("one\ntwo three")
  buffer = press(buffer, "return")!
  assert.equal(buffer.text, "one\ntwo three\n")
  buffer = { text: "one\ntwo three", cursor: 8 }
  buffer = press(buffer, "up")!
  assert.equal(buffer.cursor, 3)
  buffer = press(buffer, "down")!
  assert.equal(buffer.cursor, 7)
  assert.equal(press({ text: "ab\ncd", cursor: 4 }, "home")!.cursor, 3)
  assert.equal(press({ text: "ab\ncd", cursor: 0 }, "end")!.cursor, 2)
})

test("single line buffers do not take Enter", () => {
  assert.equal(press(bufferAtEnd("name"), "return", {}, false), null)
})

test("pasted text is normalized and loses control bytes", () => {
  assert.equal(cleanPastedText("a\r\nb\u0007\u001b[31mc\td", true), "a\nb[31mc\td")
  assert.equal(cleanPastedText("a\nb", false), "ab")
})
