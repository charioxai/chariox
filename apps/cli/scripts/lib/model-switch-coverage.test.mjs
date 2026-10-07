import assert from "node:assert/strict"
import test from "node:test"
import { agentSnapshot, assertContinuedPlacement, evidenceName, placementRequest, roundTripMatrix, scoreFacts, scoreSummary } from "./model-switch-coverage.mjs"
test("OpenCode model IDs produce one evidence filename", () => {
  assert.equal(evidenceName({provider:"opencode",model:"opencode/big-pickle"}, {provider:"codex",model:"gpt-6"}, 1), "opencode-opencode-big-pickle-to-codex-gpt-6-1.json")
})
test("leased and slice prompts use the existing home-owned agent path", () => {
  const profile = {provider:"codex",model:"gpt-6",effort:"low",accountProfile:"work"}
  const remote = placementRequest("s",profile,{kernelRef:"worker"}).SpawnAgent
  assert.equal(remote.kernel_ref,"worker"); assert.equal(remote.slice_ref,null); assert.equal(remote.account_profile,"work")
  const slice = placementRequest("s",profile,{sliceRef:"slice"}).SpawnAgent
  assert.equal(slice.slice_ref,"slice"); assert.equal(slice.kernel_ref,null)
  assert.throws(() => placementRequest("s",profile,{kernelRef:"worker",sliceRef:"slice"}))
})
test("all provider directions and each internal downshift have a return leg", () => {
  const profiles = Object.fromEntries(["codex","claude","opencode"].map(provider=>[provider,{primary:`${provider}:large`,alternate:`${provider}:small`}]))
  const cases = roundTripMatrix(profiles)
  assert.equal(cases.length,9); assert.equal(new Set(cases.map(c=>c.id)).size,9)
  assert.deepEqual(cases.find(c=>c.id === "opencode-to-claude-to-opencode"), {id:"opencode-to-claude-to-opencode",from:"opencode:large",to:"claude:large"})
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
