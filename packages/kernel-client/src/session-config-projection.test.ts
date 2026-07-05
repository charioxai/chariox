import assert from "node:assert/strict"
import test from "node:test"

import {
  normalizeSessionResponseLayout,
  sessionResponseLayout,
  SESSION_CONFIG_RESPONSE_LAYOUT_KEY,
} from "./session-config-projection.js"
import { makeSession } from "./shell-executor.test-support.js"

test("session response layout prefers session config over fallback", () => {
  const splitSession = makeSession({
    config_state: {
      version: 1,
      values: {
        [SESSION_CONFIG_RESPONSE_LAYOUT_KEY]: "split",
      },
      updated_by_attachment_id: null,
    },
  })

  assert.equal(sessionResponseLayout(splitSession, "individual"), "split")
  assert.equal(sessionResponseLayout(makeSession(), "split"), "split")
  assert.equal(sessionResponseLayout(makeSession(), null), "individual")
})

test("session response layout ignores unknown values", () => {
  assert.equal(normalizeSessionResponseLayout(" split "), null)
  assert.equal(normalizeSessionResponseLayout("grid"), null)
  assert.equal(normalizeSessionResponseLayout("individual"), "individual")
  assert.equal(normalizeSessionResponseLayout("split"), "split")
  assert.equal(sessionResponseLayout(makeSession({
    config_state: {
      version: 1,
      values: {
        [SESSION_CONFIG_RESPONSE_LAYOUT_KEY]: "grid",
      },
      updated_by_attachment_id: null,
    },
  }), "split"), "split")
})
