import assert from "node:assert/strict"
import test from "node:test"
import { dprFrom } from "./args.mjs"

test("--dpr defaults to 1 only when absent and rejects anything but 1 or 2", () => {
  assert.equal(dprFrom(["node", "drill.mjs", "547"]), 1)
  assert.equal(dprFrom(["node", "drill.mjs", "547", "--dpr", "2"]), 2)
  for (const bad of [["--dpr"], ["--dpr", "3"], ["--dpr", "x"]]) assert.throws(() => dprFrom(["node", "drill.mjs", "547", ...bad]))
})
