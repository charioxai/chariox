import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance } from "./kernel-types.js"
import { createDefaultShellContext, parseShellCommand } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"
import {
  fakeClient,
  makeAgent,
  makeSession,
} from "./shell-executor.test-support.js"

export {
  assert,
  createDefaultShellContext,
  executeShellCommand,
  fakeClient,
  makeAgent,
  makeSession,
  parseShellCommand,
  test,
}
export type { AgentInstance }
