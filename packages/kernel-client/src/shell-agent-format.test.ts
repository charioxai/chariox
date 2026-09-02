import assert from "node:assert/strict"
import test from "node:test"

import { makeAgent, makeSession } from "./shell-executor.test-support.js"
import {
  formatAgentInspectSummary,
  formatAgentListSummary,
  formatAgentSubstituteSummary,
} from "./shell-agent-format.js"

test("formatAgentSubstituteSummary shows the selected account via a public label", () => {
  const agent = makeAgent({
    id: "agent-1",
    agent_ref: "agent-1",
    active_substitute_index: 1,
    substitutes: [
      { provider: "codex", model: "gpt-5.4" },
      { provider: "codex", model: "gpt-5.4", variant: "high", account_profile: "codex-work-internal" },
    ],
  })

  const rendered = formatAgentSubstituteSummary(agent, (provider, accountProfile) =>
    provider === "codex" && accountProfile === "codex-work-internal" ? "Work" : null)

  assert.match(rendered, /- 0: codex\/gpt-5\.4\n/)
  assert.match(rendered, /\* 1: codex\/gpt-5\.4\/high · account Work/)
  assert.doesNotMatch(rendered, /codex-work-internal/)
})

test("formatAgentSubstituteSummary stays honest when account inventory is unavailable", () => {
  const agent = makeAgent({
    id: "agent-1",
    agent_ref: "agent-1",
    substitutes: [
      { provider: "codex", model: "gpt-5.4", account_profile: "codex-work-internal" },
    ],
  })

  const rendered = formatAgentSubstituteSummary(agent)

  assert.match(rendered, /custom account/)
  assert.doesNotMatch(rendered, /codex-work-internal/)
})

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
    session: makeSession({
      agents: [remoteAgent],
      agent_activity: {
        "agent-remote": {
          status: "idle",
          prompt_status: "none",
          busy: false,
          unread_idle_output: false,
          active_turn: null,
        },
      },
    }),
  })

  assert.match(rendered, /agent-remote \[Idle;/)
  assert.doesNotMatch(rendered, /provider blocked/)
})

test("formatAgentInspectSummary uses projected idle activity over stale legacy worker state", () => {
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

  const rendered = formatAgentInspectSummary(remoteAgent, [], null, {}, {
    session: makeSession({
      agents: [remoteAgent],
      agent_activity: {
        "agent-remote": {
          status: "idle",
          prompt_status: "none",
          busy: false,
          unread_idle_output: false,
          active_turn: null,
        },
      },
    }),
  })

  assert.match(rendered, /^agent-remote \[Idle\]/)
  assert.doesNotMatch(rendered, /provider run next:/)
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
    session: makeSession({
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
        },
      },
    }),
  })

  assert.match(rendered, /provider run: none/)
  assert.match(rendered, /^agent-remote \[Working\]/)
  assert.match(rendered, /provider run next: run \/kernel remote-runtime and \/machine kernels worker-machine/)
})

test("formatAgentListSummary uses session-scoped runtime activity when session is supplied", () => {
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
    session: makeSession({
      agents: [remoteAgent],
      agent_activity: {
        "agent-ghost": {
          status: "working",
          prompt_status: "running",
          busy: true,
          unread_idle_output: false,
        },
      },
    }),
  })

  assert.match(rendered, /agent-remote \[Idle;/)
  assert.doesNotMatch(rendered, /provider blocked/)
})
