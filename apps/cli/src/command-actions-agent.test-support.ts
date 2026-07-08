import assert from "node:assert/strict"
import test from "node:test"

import { createCommandActionHandlers } from "./command-actions.js"
import type { AgentInstance, RuntimeAttachment, RuntimeProviderRun, RuntimeSession } from "./cli-types.js"
import { makeAgent, makeCommandDeps, makeSession } from "./command-actions-test-support.js"

export {
  assert,
  createCommandActionHandlers,
  makeAgent,
  makeCommandDeps,
  makeSession,
  test,
}
export type { AgentInstance, RuntimeAttachment, RuntimeProviderRun, RuntimeSession }
