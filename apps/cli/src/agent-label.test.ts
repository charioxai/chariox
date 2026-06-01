import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, SliceRecord } from "./cli-types.js"
import { formatAgentLabel, formatAgentLocationLabel } from "./agent-label.js"

test("formatAgentLabel includes alias when present", () => {
  assert.equal(formatAgentLabel(agent({ alias: "Builder" })), "agent-a (Builder)")
})

test("formatAgentLabel falls back to an empty label without an agent", () => {
  assert.equal(formatAgentLabel(null), "")
})

test("formatAgentLocationLabel prefers matching slice labels", () => {
  assert.equal(
    formatAgentLocationLabel(
      agent({
        remote_execution: {
          worker_kernel_id: "kernel-1",
          worker_machine_id: "machine-1",
          execution_lease_id: "lease-1",
          leased_agent_id: "leased-1",
        },
      }),
      [slice({ worker_kernel_ref: "kernel-1", name: "builder" })],
    ),
    "slice:builder",
  )
})

test("formatAgentLocationLabel prefers explicit agent slice bindings", () => {
  assert.equal(
    formatAgentLocationLabel(
      agent({
        id: "agent-2",
        remote_execution: {
          worker_kernel_id: "kernel-1",
          worker_machine_id: "machine-1",
          execution_lease_id: "lease-1",
          leased_agent_id: "leased-1",
        },
      }),
      [
        slice({ worker_kernel_ref: "kernel-1", name: "wrong", agent_ids: ["agent-1"] }),
        slice({ worker_kernel_ref: "kernel-1", name: "right", agent_ids: ["agent-2"] }),
      ],
    ),
    "slice:right",
  )
})

test("formatAgentLocationLabel falls back to remote kernel labels", () => {
  assert.equal(formatAgentLocationLabel(agent(), []), null)
  assert.equal(
    formatAgentLocationLabel(
      agent({
        remote_execution: {
          worker_kernel_id: "kernel-1",
          worker_machine_id: "machine-1",
          execution_lease_id: "lease-1",
          leased_agent_id: "leased-1",
        },
      }),
      [],
    ),
    "remote:kernel-1",
  )
})

function agent(overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id: "agent-1",
    agent_ref: "agent-a",
    session_id: "session-1",
    alias: null,
    provider: "codex",
    model: "gpt-5",
    worktree_id: null,
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 1,
    last_activity_at_ms: 1,
    ...overrides,
  }
}

function slice(overrides: Partial<SliceRecord> = {}): SliceRecord {
  return {
    id: "slice-1",
    name: "",
    owner_kernel_id: "owner-kernel",
    owner_machine_id: "owner-machine",
    backend: "local_docker",
    os: "linux",
    status: "running",
    worker_kernel_ref: "kernel-ref",
    created_at_ms: 1,
    updated_at_ms: 1,
    ...overrides,
  }
}
