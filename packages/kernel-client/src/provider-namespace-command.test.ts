import assert from "node:assert/strict"
import test from "node:test"

import {
  parseProviderNamespaceCommand,
  providerNamespace,
  providerSupportsNamespaceCommands,
} from "./provider-namespace-command.js"

test("provider namespace helpers expose supported provider namespaces", () => {
  assert.equal(providerNamespace("codex"), "/codex")
  assert.equal(providerSupportsNamespaceCommands("codex"), true)
  assert.equal(providerSupportsNamespaceCommands("opencode"), true)
  assert.equal(providerSupportsNamespaceCommands("claude-p"), false)
  assert.equal(providerSupportsNamespaceCommands(null), false)
})

test("parseProviderNamespaceCommand rewrites supported namespaces", () => {
  assert.deepEqual(parseProviderNamespaceCommand("/opencode compact"), {
    raw: "/opencode compact",
    provider: "opencode",
    forwardedCommand: "/compact",
  })
  assert.deepEqual(parseProviderNamespaceCommand("/codex /model gpt-5"), {
    raw: "/codex /model gpt-5",
    provider: "codex",
    forwardedCommand: "/model gpt-5",
  })
})

test("parseProviderNamespaceCommand preserves empty supported commands and ignores others", () => {
  assert.deepEqual(parseProviderNamespaceCommand(" /codex "), {
    raw: "/codex",
    provider: "codex",
    forwardedCommand: "",
  })
  assert.equal(parseProviderNamespaceCommand("/claude compact"), null)
  assert.equal(parseProviderNamespaceCommand("/codexical compact"), null)
})
