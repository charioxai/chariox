import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
} from "../cli-types.js"
import {
  formatNativeTuiRuntimeBanner,
} from "./runtime-banner.js"

test("native TUI runtime banner shows ownership, placement, worktree, and live sync", () => {
  const banner = formatNativeTuiRuntimeBanner({
    surface: "codex native-tui",
    session: session({
      alias: "Review",
      host_daemon_id: "home-kernel",
      host_machine_id: "home-machine",
      workspace_live_sync_mode: "tracked",
    }),
    agent: agent({
      alias: "builder",
      remote_execution: {
        worker_kernel_id: "worker-kernel",
        worker_machine_id: "hetzner",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
        active_worker_provider_run_id: "worker-run-1",
      },
    }),
    worktree: "/repo/worktrees/feature",
    run: providerRun("session-run-1"),
    providerLines: ["  proxy:          ws://127.0.0.1:1234"],
    promptPolicy: "native prompts pass through",
  })

  assert.match(banner, /^\[arroba codex native-tui\]/)
  assert.match(banner, /arroba session: session-1 \(Review\)/)
  assert.match(banner, /arroba agent:   agent-1 \(builder\)/)
  assert.match(banner, /home kernel:    home-kernel@home-machine/)
  assert.match(banner, /worktree:       \/repo\/worktrees\/feature/)
  assert.match(banner, /placement:      remote \(worker=hetzner, kernel=worker-kernel, lease=lease-1, leased_agent=leased-agent-1, active_run=worker-run-1\)/)
  assert.match(banner, /live sync:      tracked/)
  assert.match(banner, /provider run:   session-run-1/)
  assert.match(banner, /proxy:          ws:\/\/127\.0\.0\.1:1234/)
  assert.match(banner, /prompt policy:  native prompts pass through/)
})

test("native TUI runtime banner renders local defaults without inventing state", () => {
  const banner = formatNativeTuiRuntimeBanner({
    surface: "opencode native-tui",
    session: session({ workspace_live_sync_mode: null }),
    agent: agent({ remote_execution: null }),
    worktree: "",
  })

  assert.match(banner, /home kernel:    -/)
  assert.match(banner, /worktree:       \/repo/)
  assert.match(banner, /placement:      worker-local/)
  assert.match(banner, /live sync:      config default/)
  assert.doesNotMatch(banner, /provider run:/)
})

function session(overrides: Partial<RuntimeSession>): RuntimeSession {
  return {
    id: "session-1",
    workspace_id: "/repo",
    worktree_id: "/repo",
    created_at_ms: 1,
    status: "Active",
    active_provider_run_id: null,
    attachment_ids: [],
    active_prompt: null,
    queued_prompts: [],
    focused_agent_id: "agent-1",
    max_agents: 6,
    agents: [],
    workflows: [],
    workflow_runs: [],
    workflow_watchdogs: [],
    workflow_consoles: [],
    config_state: {
      version: 1,
      values: {},
      updated_by_attachment_id: null,
    },
    ...overrides,
  }
}

function agent(overrides: Partial<AgentInstance>): AgentInstance {
  return {
    id: "agent-1",
    agent_ref: "A1",
    session_id: "session-1",
    alias: null,
    provider: "codex",
    model: "gpt-5.3-codex",
    worktree_id: "/repo",
    remote_execution: null,
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

function providerRun(id: string): RuntimeProviderRun {
  return {
    id,
    session_id: "session-1",
    agent_instance_id: "agent-1",
    adapter_key: "codex",
    provider: "codex",
    account_profile: "default",
    model: "gpt-5.3-codex",
    variant: null,
    usage_tokens_total: null,
    state: "Ready",
  }
}
