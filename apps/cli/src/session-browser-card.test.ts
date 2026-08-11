import assert from "node:assert/strict"
import test from "node:test"

import { sessionBrowserCardLines } from "./session-browser-card.js"
import { sessionBrowserFixture } from "./session-browser-card.test-fixture.js"

test("session browser card places nonzero ERROR before optional metadata", () => {
  const lines = sessionBrowserCardLines(sessionBrowserFixture, true)
  assert.ok(lines.agents.indexOf("1 ERROR") < 80)
  assert.match(lines.agents, /^\s+3 agents · 2 DONE · 1 ERROR/)
  assert.equal(lines.workflows, "  1 workflows")
  assert.equal(lines.collaborations, "  1 collaborators joined · 1 invitations pending")
})
