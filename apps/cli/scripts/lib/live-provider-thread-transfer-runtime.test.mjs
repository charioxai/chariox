import assert from "node:assert/strict"
import test from "node:test"

import { cleanupSliceRuntime } from "./live-provider-thread-transfer-slice-scenarios.mjs"
import { terminalProviderHistoryError } from "./live-provider-thread-transfer-runtime.mjs"

test("slice restart cleanup resets saved state before deleting the slice", async () => {
  const requests = []
  const evidence = {}
  const client = {
    async send(request) {
      requests.push(request)
      if ("ResetSliceState" in request) throw new Error("saved state cleanup failed")
      return { SliceDeleted: { slice: { id: "slice-1" } } }
    },
  }

  await cleanupSliceRuntime(client, "slice-1", evidence, { resetSavedState: true })

  assert.deepEqual(requests, [
    { ResetSliceState: { slice_ref: "slice-1" } },
    { DeleteSlice: { slice_ref: "slice-1" } },
  ])
  assert.equal(evidence.slice_state_cleanup_error, "saved state cleanup failed")
  assert.equal(evidence.slice_cleanup_error, undefined)
})

test("provider thread transfer fails fast on terminal provider history", () => {
  const failure = terminalProviderHistoryError([
    { kind: "notice", text: "provider is starting" },
    { kind: "provider_error", text: "account balance exhausted" },
  ])

  assert.equal(failure?.text, "account balance exhausted")
})

test("provider thread transfer ignores nonterminal provider history", () => {
  assert.equal(terminalProviderHistoryError([
    { kind: "notice", text: "provider is starting" },
    { kind: "provider_output", text: "done" },
  ]), null)
})
