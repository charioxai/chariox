import assert from "node:assert/strict"
import test from "node:test"

import {
  providerRunRecoveryActions,
  remoteWorkerProviderRunRecoveryAction,
} from "./provider-run-recovery.js"

test("providerRunRecoveryActions reports mismatched session provider runs", () => {
  assert.deepEqual(providerRunRecoveryActions({
    agent: { id: "agent-1", agent_ref: "A1" },
    activeProviderRunId: "run-1",
    activeProviderRunAgentId: "agent-2",
  }), [
    "run /kernel health and /provider processes; export a debug bundle, then close or relaunch the mismatched provider run before sending more prompts to A1",
  ])
})

test("providerRunRecoveryActions reports missing active remote worker runs", () => {
  assert.deepEqual(providerRunRecoveryActions({
    agent: {
      id: "agent-1",
      agent_ref: "A1",
      state: "Working",
      remote_execution: {
        worker_machine_id: "hetzner",
      },
    },
  }), [
    "run /kernel remote-runtime and /machine kernels hetzner; reconnect or relaunch the remote/slice worker before sending prompts to that remote/slice agent if no active worker run appears",
  ])
})

test("providerRunRecoveryActions stays quiet for healthy local and remote runs", () => {
  assert.deepEqual(providerRunRecoveryActions({
    agent: { id: "agent-1", agent_ref: "A1" },
    activeProviderRunId: "run-1",
    activeProviderRunAgentId: "agent-1",
  }), [])
  assert.deepEqual(providerRunRecoveryActions({
    agent: {
      id: "agent-1",
      agent_ref: "A1",
      state: "Working",
      remote_execution: {
        worker_machine_id: "hetzner",
        active_worker_provider_run_id: "worker-run-1",
      },
    },
  }), [])
})

test("remoteWorkerProviderRunRecoveryAction formats specific and fallback actions", () => {
  assert.equal(
    remoteWorkerProviderRunRecoveryAction("A1", "hetzner"),
    "run /kernel remote-runtime; run /agent inspect A1; run /machine kernels hetzner; reconnect or relaunch the remote/slice worker before sending prompts to that remote/slice agent",
  )
  assert.equal(
    remoteWorkerProviderRunRecoveryAction(null, null),
    "run /kernel remote-runtime; identify the affected remote/slice agent and worker before sending prompts to that agent",
  )
  assert.equal(
    remoteWorkerProviderRunRecoveryAction("<agent>", "<worker-machine>"),
    "run /kernel remote-runtime; identify the affected remote/slice agent and worker before sending prompts to that agent",
  )
})
