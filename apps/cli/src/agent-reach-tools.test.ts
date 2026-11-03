import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import test from "node:test"
import { reachToolProblems, reachToolsDocument, REACH_TOOLS } from "./agent-reach-tools.js"

test("the chariox-reach tools match the command table", () => {
  assert.deepEqual(reachToolProblems(), [])
  assert.deepEqual(REACH_TOOLS.map(tool => tool.name), ["kernels", "sessions", "request", "events_subscribe", "events_wait", "events_ack", "guide"])
})

test("the tool list the kernel embeds is the one the command table generates", () => {
  const embedded = readFileSync(new URL("../../../apps/kernel/src/transport/runtime_tools/reach_tools.json", import.meta.url), "utf8")
  assert.equal(embedded, JSON.stringify(reachToolsDocument(), null, 2) + "\n", "run pnpm reach:generate")
})

test("the reach guide names every tool and none of the CLI's access steps", () => {
  const { guide } = reachToolsDocument()
  for (const tool of REACH_TOOLS) assert.match(guide, new RegExp(`\\b${tool.name}\\(`))
  assert.doesNotMatch(guide, /access request|RequestKernelSudo|RespondToInteraction/)
})
