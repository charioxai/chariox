import assert from "node:assert/strict"
import test from "node:test"

import { parseDrillMaxDepth } from "./drill-cli-args.mjs"

test("parses drill max-depth arguments", () => {
  assert.equal(parseDrillMaxDepth("0"), 0)
  assert.equal(parseDrillMaxDepth("8"), 8)
  assert.throws(() => parseDrillMaxDepth("-1"), /non-negative integer/)
  assert.throws(() => parseDrillMaxDepth("1.5"), /non-negative integer/)
  assert.throws(() => parseDrillMaxDepth("nope"), /non-negative integer/)
})
