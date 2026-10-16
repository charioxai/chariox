// MP-08 / MP-10 / MP-11: all execution placements use the kernel's account and hook.
import assert from "node:assert/strict"
import test from "node:test"
import { parseNativeClaudeArgs } from "./claude.js"
for (const args of [[], ["--machine", "worker"], ["--slice", "slice-a"]]) {
  test(`native Claude uses kernel-owned PTY/account/hook for ${args.join(" ") || "local"}`, () => {
    assert.equal(parseNativeClaudeArgs(args).remoteRendered, true)
  })
}
