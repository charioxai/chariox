// Row 324: `chariox claude` takes its default model from the kernel, never from a list of its own.
import assert from "node:assert/strict"
import test from "node:test"
import { parseNativeClaudeArgs } from "./claude.js"

test("M1: no --model leaves the model to the kernel's own default", () => {
  assert.equal(parseNativeClaudeArgs([]).model, undefined)
  assert.equal(parseNativeClaudeArgs(["some-session"]).model, undefined)
})

test("M2: an explicit --model or -m is kept", () => {
  assert.equal(parseNativeClaudeArgs(["--model", "claude-opus-5-5"]).model, "claude-opus-5-5")
  assert.equal(parseNativeClaudeArgs(["-m", "claude-haiku-5-5"]).model, "claude-haiku-5-5")
})

test("M1: --help names no hard-coded model id", () => {
  const originalWrite = process.stdout.write
  const originalExit = process.exit
  let output = ""
  process.stdout.write = ((chunk: string | Uint8Array) => { output += String(chunk); return true }) as typeof process.stdout.write
  process.exit = (() => { throw new Error("exit") }) as typeof process.exit
  try {
    assert.throws(() => parseNativeClaudeArgs(["--help"]), /exit/)
  } finally {
    process.stdout.write = originalWrite
    process.exit = originalExit
  }
  assert.doesNotMatch(output, /claude-(sonnet|opus|haiku|fable)-\d/)
})
