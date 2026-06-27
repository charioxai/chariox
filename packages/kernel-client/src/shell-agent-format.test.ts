import assert from "node:assert/strict"
import test from "node:test"

import { makeAgent } from "./shell-executor.test-support.js"
import {
  formatAgentInspectSummary,
  formatAgentListSummary,
} from "./shell-agent-format.js"

test("formatAgentListSummary uses projected idle activity over stale legacy worker state", () => {
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

  const rendered = formatAgentListSummary([remoteAgent], [], {}, {
    agentActivity: {
      "agent-remote": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        active_turn: null,
      },
    },
  })

  assert.match(rendered, /agent-remote \[Working;/)
  assert.doesNotMatch(rendered, /provider blocked/)
})

test("formatAgentInspectSummary uses projected busy activity for provider run recovery", () => {
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

  const rendered = formatAgentInspectSummary(remoteAgent, [], null, {}, {
    agentActivity: {
      "agent-remote": {
        status: "working",
        prompt_status: "running",
        busy: true,
        active_turn: {
          prompt_id: "prompt-1",
          status: "running",
          phase: "streaming",
        },
      },
    },
  })

  assert.match(rendered, /provider run: none/)
  assert.match(rendered, /provider run next: run \/kernel remote-runtime and \/machine kernels worker-machine/)
})
