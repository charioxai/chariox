import assert from "node:assert/strict"
import test from "node:test"
import { requireKernelControlCapability } from "./ipc-control-capability.js"

const capability = "exact_prompt_cancellation_v499"
const conditional = { CancelActivePrompt: { session_id: "s", attachment_id: "a", expected_prompt_id: "prompt-A" } }
const status = (capabilities?: unknown, daemon_id = "home-1") => ({
  RelayStatus: { status: { daemon_id, machine_id: "machine-1", capabilities } },
})

test("conditional prompt cancellation rejects peers without the capability", async () => {
  for (const protocol_version of [351, 365]) {
    await assert.rejects(requireKernelControlCapability(async () => ({
      ...status(), protocol_version,
    }), conditional), /does not support/)
  }
})

test("capability admission binds the connected home and expected machine", async () => {
  await requireKernelControlCapability(async () => status([capability]), conditional,
    { daemonId: "home-1", machineId: "machine-1" })
  await assert.rejects(requireKernelControlCapability(async () => status([capability], "other"), conditional,
    { daemonId: "home-1" }), /another kernel/)
  await assert.rejects(requireKernelControlCapability(async () => status([capability]), conditional,
    { daemonId: "home-1", machineId: "other" }), /another machine/)
})

test("the capability is read from fresh status each time", async () => {
  let calls = 0
  const send = async (query: unknown) => {
    assert.deepEqual(query, { RelayStatus: null })
    return status(calls++ === 0 ? [capability] : [])
  }
  await requireKernelControlCapability(send, conditional)
  await assert.rejects(requireKernelControlCapability(send, conditional), /does not support/)
  assert.equal(calls, 2)
})

test("unrelated requests do not add a preflight; malformed status fails closed", async () => {
  await requireKernelControlCapability(async () => { throw new Error("unexpected query") }, { RelayStatus: null })
  for (const response of [null, {}, { Error: "offline" }, status(capability), status(["unknown"]), status([capability, null])]) {
    await assert.rejects(requireKernelControlCapability(async () => response, conditional))
  }
})

// MP-08 / MP-10 / MP-11: old kernels silently ignore optional prompt fences.
test("conditional prompt cancellation requires its own fresh capability", async () => {
  for (const capabilities of [undefined, [], ["unknown"]]) {
    await assert.rejects(requireKernelControlCapability(async () => status(capabilities), conditional), new RegExp(capability))
  }
  await requireKernelControlCapability(async () => status([capability]), conditional)
  await requireKernelControlCapability(async () => { throw new Error("legacy cancel must not preflight") },
    { CancelActivePrompt: { session_id: "s", attachment_id: "a" } })
  await assert.rejects(requireKernelControlCapability(async () => status([]),
    { CancelActivePrompt: { session_id: "s", attachment_id: "a", expected_prompt_id: "" } }), new RegExp(capability))
})
