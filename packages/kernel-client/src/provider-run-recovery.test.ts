import assert from "node:assert/strict"
import test from "node:test"

import {
  providerRunRecoveryActions,
  remoteWorkerProviderRunIsMissing,
  remoteWorkerProviderRunRecoveryAction,
  sessionCanIgnoreMissingActiveProviderRun,
  sessionNeedsAttachedRuntimeCatchUp,
  sessionShouldRecoverMissingActiveProviderRun,
} from "./provider-run-recovery.js"
import {
  makeAgent,
  makeSession,
} from "./shell-executor.test-support.js"

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

test("providerRunRecoveryActions uses projected busy state when supplied", () => {
  const agent = {
    id: "agent-1",
    agent_ref: "A1",
    state: "Working",
    is_processing: true,
    remote_execution: {
      worker_machine_id: "hetzner",
    },
  }

  assert.deepEqual(providerRunRecoveryActions({
    agent,
    agentBusy: false,
  }), [])

  assert.deepEqual(providerRunRecoveryActions({
    agent: {
      ...agent,
      state: "Idle",
      is_processing: false,
    },
    agentBusy: true,
  }), [
    "run /kernel remote-runtime and /machine kernels hetzner; reconnect or relaunch the remote/slice worker before sending prompts to that remote/slice agent if no active worker run appears",
  ])
})

test("remoteWorkerProviderRunIsMissing shares legacy and projected busy policy", () => {
  const agent = {
    id: "agent-1",
    agent_ref: "A1",
    state: "Working",
    is_processing: true,
    remote_execution: {
      worker_machine_id: "hetzner",
    },
  }

  assert.equal(remoteWorkerProviderRunIsMissing({ agent }), true)
  assert.equal(remoteWorkerProviderRunIsMissing({ agent, agentBusy: false }), false)
  assert.equal(remoteWorkerProviderRunIsMissing({
    agent: {
      ...agent,
      state: "Idle",
      is_processing: false,
      remote_execution: {
        worker_machine_id: "hetzner",
        active_worker_provider_run_id: "worker-run-1",
      },
    },
    agentBusy: true,
  }), false)
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

test("session provider run recovery policy catches up active or prompt-owned work", () => {
  const idle = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
  })
  const activeProviderRun = makeSession({
    active_provider_run_id: "run-1",
    agents: [makeAgent({ id: "agent-1" })],
  })
  const activePromptWithoutProviderRun = makeSession({
    agents: [makeAgent({ id: "agent-1" })],
    prompt_states: {
      "agent-1": {
        active_prompt: {
          id: "prompt-1",
          source_attachment_id: "attach-1",
          target_agent_id: "agent-1",
          prompt: "run",
          status: "Running",
        },
        queued_prompts: [],
      },
    },
  })

  assert.equal(sessionNeedsAttachedRuntimeCatchUp(idle), false)
  assert.equal(sessionNeedsAttachedRuntimeCatchUp(activeProviderRun), true)
  assert.equal(sessionNeedsAttachedRuntimeCatchUp(activePromptWithoutProviderRun), true)

  assert.equal(sessionShouldRecoverMissingActiveProviderRun(idle), false)
  assert.equal(sessionCanIgnoreMissingActiveProviderRun(idle), true)
  assert.equal(sessionShouldRecoverMissingActiveProviderRun(activePromptWithoutProviderRun), true)
  assert.equal(sessionCanIgnoreMissingActiveProviderRun(activePromptWithoutProviderRun), false)
})
