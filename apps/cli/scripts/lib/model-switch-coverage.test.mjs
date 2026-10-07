import assert from "node:assert/strict"
import test from "node:test"
import { agentSnapshot, assertContinuedPlacement, assertToolProbe, evidenceName, scoreFacts, scoreSummary } from "./model-switch-coverage.mjs"
test("OpenCode model IDs produce one evidence filename", () => {
  assert.equal(evidenceName({provider:"opencode",model:"opencode/big-pickle"}, {provider:"codex",model:"gpt-6"}, 1), "opencode-opencode-big-pickle-to-codex-gpt-6-1.json")
})
test("recall scoring stays exact per key and retains the round-four totals", () => {
  const facts = {codename:"amber-kestrel",port:"31234",created_file:"ctx-amber.txt"}
  assert.deepEqual(scoreSummary(scoreFacts("codename=amber-kestrel port=31234 created_file=ctx-amber.txt",facts)),{correct:3,total:3})
  assert.deepEqual(scoreSummary(scoreFacts("codename=UNKNOWN amber-kestrel port=312345 created_file=other",facts)),{correct:1,total:3})
})

test("evidence retains placement identity without transport credentials", () => {
  const source = agentSnapshot({provider:"codex", remote_execution:{worker_kernel_id:"worker",execution_lease_id:"lease",leased_agent_id:"agent",relay_token:"fixture-private",relay_url:"ws://relay"}})
  assert.equal(JSON.stringify(source).includes("fixture-private"),false)
  assertContinuedPlacement(source,source,{kernelRef:"worker"})
  assert.throws(()=>assertContinuedPlacement(source,{remote_execution:{...source.remote_execution,leased_agent_id:"other"}},{sliceRef:"slice"}))
  assert.throws(()=>assertContinuedPlacement({},source,{kernelRef:"worker"}))
})

test("file probes require a real tool and its verified output", () => {
  assert.throws(() => assertToolProbe({lifecycle:"completed",text:"DONE",tool_rows:[]},"amber-kestrel","file"))
  assert.throws(() => assertToolProbe({lifecycle:"completed",text:"amber-kestrel",tool_rows:[]},"amber-kestrel","file"))
  assert.throws(() => assertToolProbe({lifecycle:"failed",text:"amber-kestrel",tool_rows:["read file"]},"amber-kestrel","file"))
  assertToolProbe({lifecycle:"completed",text:"amber-kestrel\n",tool_rows:["read file"]},"amber-kestrel","file")
})
