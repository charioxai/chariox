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
