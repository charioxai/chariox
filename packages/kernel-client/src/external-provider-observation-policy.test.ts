import assert from "node:assert/strict"
import test from "node:test"

import {
  externalProviderStatusIsPassiveTelemetry,
} from "./external-provider-observation-policy.js"

test("external provider observation policy classifies passive status telemetry by provider", () => {
  assert.equal(externalProviderStatusIsPassiveTelemetry(" Codex ", " CODEX token_count\n{}"), true)
  assert.equal(externalProviderStatusIsPassiveTelemetry("claude", " Claude last-prompt {\"lastPrompt\":\"hello\"}"), true)
  assert.equal(externalProviderStatusIsPassiveTelemetry("claude", " CLAUDE ai-title {\"title\":\"x\"}"), true)
  assert.equal(externalProviderStatusIsPassiveTelemetry("opencode", "opencode message completed"), false)
  assert.equal(externalProviderStatusIsPassiveTelemetry("codex", "codex task_complete\n{}"), false)
  assert.equal(externalProviderStatusIsPassiveTelemetry(null, "codex token_count\n{}"), false)
})
