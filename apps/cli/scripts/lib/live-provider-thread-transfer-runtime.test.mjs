import assert from "node:assert/strict"
import test from "node:test"

import { terminalProviderHistoryError } from "./live-provider-thread-transfer-runtime.mjs"

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
