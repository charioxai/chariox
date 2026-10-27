import assert from "node:assert/strict"
import test from "node:test"

import { makeAgent, makeSession } from "./shell-executor.test-support.js"
import { formatSessionList } from "./shell-session-format.js"

test("formatSessionList uses projected idle activity over stale legacy worker state", () => {
  const remoteAgent = makeAgent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    state: "Working",
    is_processing: true,
    remote_execution: {
      worker_kernel_id: "slice:slice-1",
      worker_machine_id: "worker-machine",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
  })
  const rendered = formatSessionList([makeSession({
    id: "session-remote",
    agents: [remoteAgent],
    agent_activity: {
      "agent-remote": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
        active_turn: null,
        last_completed_turn: null,
      },
    },
    agent_activity_revision: 3,
  })])

  assert.match(rendered, /remote 1 agent, 1 slice/)
  assert.doesNotMatch(rendered, /worker run gap/)
})

test("formatSessionList uses projected busy activity for remote worker run gaps", () => {
  const remoteAgent = makeAgent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    state: "Idle",
    is_processing: false,
    remote_execution: {
      worker_kernel_id: "slice:slice-1",
      worker_machine_id: "worker-machine",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
  })
  const rendered = formatSessionList([makeSession({
    id: "session-remote",
    agents: [remoteAgent],
    agent_activity: {
      "agent-remote": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-1",
          status: "running",
          phase: "streaming",
        },
        last_completed_turn: null,
      },
    },
    agent_activity_revision: 3,
  })])

  assert.match(rendered, /remote 1 agent, 1 slice, 1 worker run gap/)
  assert.match(rendered, /next run \/kernel remote-runtime; run \/agent inspect agent-remote; run \/machine kernels worker-machine/)
})
