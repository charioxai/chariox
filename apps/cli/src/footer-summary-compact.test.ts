import assert from "node:assert/strict"
import test from "node:test"

import { compactFooterSummary } from "./footer-summary-compact.js"
import { SESSION_NEW_FOOTER_HINT } from "./sessions.js"

test("compactFooterSummary leaves summaries alone when they fit", () => {
  const summary = "Session main • 1 CLI connected • 1 visible agent • Ctrl+T hotkeys"

  assert.equal(compactFooterSummary(summary, 120), summary)
})

test("compactFooterSummary compresses attached session action hints for narrow terminals", () => {
  const summary = "Session feature-refactor • 2 CLIs connected • 2 visible agents • Ctrl+C to stop • Tab cycles focus • Ctrl+P opens workflow • Ctrl+T hotkeys"
  const compact = compactFooterSummary(summary, 80)

  assert.equal(compact, "Session feature-refactor • 2 CLIs • 2 agents • Ctrl+C stop • Ctrl+T keys")
  assert.ok(compact.length <= 80)
})

test("compactFooterSummary compresses waiting room hints for narrow terminals", () => {
  const compact = compactFooterSummary(SESSION_NEW_FOOTER_HINT, 80)

  assert.equal(compact, "Waiting room • arrows • Enter • A archive • D delete • Ctrl+T keys")
  assert.ok(compact.length <= 80)
})

test("compactFooterSummary truncates oversized session labels as a last resort", () => {
  const summary = "Session extremely-long-session-alias-that-cannot-fit-in-the-available-terminal-footer-space • Ctrl+T hotkeys"
  const compact = compactFooterSummary(summary, 42)

  assert.equal(compact, "Session extremely-long-session-alias-th...")
  assert.equal(compact.length, 42)
})
