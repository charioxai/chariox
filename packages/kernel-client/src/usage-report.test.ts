import test from "node:test"
import assert from "node:assert/strict"
import { getSessionUsageRequest } from "./ipc-recall-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { formatSessionUsage } from "./usage-report.js"
test("MP-08 / MP-10 / MP-11: shared accounting request is bound to local448", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 448)
  assert.deepEqual(getSessionUsageRequest("s"), { GetSessionUsage: { session_id: "s" } })
})
test("MP-08 / MP-10 / MP-11: unavailable counters and price are explicit", () => {
  const usage = { input_tokens: null, cached_input_tokens: null, output_tokens: null, reasoning_tokens: null, cache_write: null, cache_write_5m: null, cache_write_1h: null }
  const text = formatSessionUsage({ session_id: "s", turns: [], agents: {}, delegation_trees: {}, total: { turns: 1, unavailable_turns: 1, usage, api_equivalent_nanodollars: null } })
  assert.match(text, /input unavailable/); assert.match(text, /unavailable API-equivalent USD/)
})

test("MP-08 / MP-10 / MP-11: cost formatting preserves integer nanodollars", () => {
  const usage = { input_tokens: 1, cached_input_tokens: 0, output_tokens: 1, reasoning_tokens: 0, cache_write: null, cache_write_5m: null, cache_write_1h: null }
  const text = formatSessionUsage({ session_id: "s", turns: [], agents: {}, delegation_trees: {}, total: { turns: 1, unavailable_turns: 0, usage, api_equivalent_nanodollars: "9007199254740993" } })
  assert.match(text, /\$9007199\.254740993/)
})
