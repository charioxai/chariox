import assert from "node:assert/strict"
import test from "node:test"

import { parseNativeOpenCodeArgs } from "./opencode.js"

test("native OpenCode accepts an exact model override", () => {
  const options = parseNativeOpenCodeArgs([
    "--model",
    "opencode/kimi-k2.7-code",
    "--server-in-kernel",
  ])

  assert.equal(options.model, "opencode/kimi-k2.7-code")
  assert.equal(options.serverInKernel, true)
})

test("native OpenCode defaults to the kernel server that installs runtime MCP", () => {
  // An externally started bare `opencode serve` has no Chariox MCP binding.
  // The kernel waits for that absent server and the native launcher stays Starting.
  const options = parseNativeOpenCodeArgs(["--model", "opencode-go/deepseek-v4.1-flash"])

  assert.equal(options.serverInKernel, true)
})
