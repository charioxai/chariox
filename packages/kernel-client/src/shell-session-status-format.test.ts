import assert from "node:assert/strict"
import test from "node:test"

import { makeAgent, makeSession } from "./shell-executor.test-support.js"
import { formatSessionRuntimeStatus } from "./shell-session-status-format.js"

test("formatSessionRuntimeStatus renders home authority, placement, sync, and recovery", () => {
  const remoteAgent = makeAgent({
    id: "agent-remote",
    agent_ref: "agent-remote",
    state: "Working",
    is_processing: true,
    remote_execution: {
      worker_kernel_id: "worker-kernel",
      worker_machine_id: "worker-machine",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
    extension_grants: [{ kind: "connector", name: "status-api" }],
    remote_extension_manifest_sync: {
      state: "failed",
      manifest_hash: "hash-1",
      pending_revoke: true,
      last_error: "worker offline",
    },
  })
  const localAgent = makeAgent({ id: "agent-local", agent_ref: "agent-local" })
  const session = makeSession({
    id: "session-1",
    alias: "release",
    workspace_id: "/repo",
    worktree_id: "/repo/main",
    owner_user_id: "alice",
    host_daemon_id: "home-kernel",
    host_machine_id: "home-machine",
    workspace_live_sync_mode: "tracked",
    attachment_ids: ["cli-1", "web-1"],
    active_provider_run_id: "run-session",
    active_prompt: { id: "prompt-1" } as never,
    queued_prompts: [{ id: "prompt-2" } as never],
    focused_agent_id: remoteAgent.id,
    agents: [localAgent, remoteAgent],
    collaboration_agent_counts: {
      collaborator_count: 1,
      owned_agent_count: 1,
      other_user_agent_count: 1,
      total_agent_count: 2,
    },
  })

  const rendered = formatSessionRuntimeStatus(session, {
    slices: [{
      id: "slice-1",
      name: "linux-dev",
      owner_kernel_id: "home-kernel",
      owner_machine_id: "home-machine",
      backend: "local_docker",
      os: "linux",
      status: "running",
      worktree_id: "/repo/main",
      worker_kernel_ref: "worker-kernel",
      worker_kernel_id: "worker-kernel",
      worker_machine_id: "worker-machine",
      agent_ids: ["agent-remote"],
      providers: ["opencode"],
      provider_auth: [{
        provider: "opencode",
        state: "authenticated",
        alias: "daily",
        email: "daily@example.com",
        source: "test",
      }],
      created_at_ms: 0,
      updated_at_ms: 0,
    }],
  })

  assert.match(rendered, /^session runtime/)
  assert.match(rendered, /session: release \(session-1\)/)
  assert.match(rendered, /home kernel: home-kernel@home-machine/)
  assert.match(rendered, /authority: home owns sessions, prompts, grants, and live sync; workers execute leases and projected tools/)
  assert.match(rendered, /live sync: tracked \(selected workspace\/worktree only; other repositories unrestricted\)/)
  assert.match(rendered, /focused agent: agent-remote/)
  assert.match(rendered, /prompts: active=1, queued=1, busy_agents=1/)
  assert.match(rendered, /agents: 2 total, 1 local, 1 remote\/slice/)
  assert.match(rendered, /remote runtime: 1 agent, 1 worker, 1 slice, 1 worker run gap/)
  assert.match(rendered, /home-proxy extensions: 1 agent, 1 sync issue, 1 pending revoke/)
  assert.match(rendered, /agent runtime:\n  - agent-local: Idle opencode\/gpt-5\.2 worktree=\/repo placement=local extensions=none/)
  assert.match(rendered, /  - agent-remote: Working opencode\/gpt-5\.2 worktree=\/repo placement=slice:linux-dev slice_status=running slice_worktree=\/repo\/main slice_auth=ready opencode slice_accounts=opencode=daily \(daily@example.com\) worker=worker-machine kernel=worker-kernel lease=lease-1 leased_agent=leased-agent-1 extensions=1 grant \(active tools home-proxy; connector=1\) manifest=failed hash=hash-1 pending_revoke=yes error=worker offline/)
  assert.match(rendered, /collaboration: 1 collaborator, 1 mine, 1 others, 2 total/)
  assert.match(rendered, /next: run \/kernel remote-runtime; run \/agent inspect agent-remote; run \/machine kernels worker-machine; reconnect or relaunch the remote\/slice worker/)
  assert.match(rendered, /next: keep the home revoke in place; run \/extension sync-status agent-remote; run \/machine kernels worker-machine if the revoke stays pending; use \/extension sync-retry agent-remote after the worker reconnects/)
})

test("formatSessionRuntimeStatus renders slice provider auth recovery", () => {
  const remoteAgent = makeAgent({
    id: "agent-slice",
    agent_ref: "agent-slice",
    remote_execution: {
      worker_kernel_id: "slice:slice-1",
      worker_machine_id: "slice-machine",
      execution_lease_id: "lease-1",
      leased_agent_id: "leased-agent-1",
    },
  })
  const rendered = formatSessionRuntimeStatus(makeSession({
    focused_agent_id: remoteAgent.id,
    agents: [remoteAgent],
  }), {
    slices: [{
      id: "slice-1",
      name: "linux-dev",
      owner_kernel_id: "home-kernel",
      owner_machine_id: "home-machine",
      backend: "local_docker",
      os: "linux",
      status: "running",
      worker_kernel_ref: "slice:slice-1",
      worker_kernel_id: "slice:slice-1",
      worker_machine_id: "slice-machine",
      agent_ids: ["agent-slice"],
      providers: ["codex"],
      provider_auth: [],
      created_at_ms: 0,
      updated_at_ms: 0,
    }],
  })

  assert.match(rendered, /slice_auth=missing codex/)
  assert.match(rendered, /next: run \/slice doctor linux-dev; configure missing provider account codex with \/slice auth import linux-dev codex or \/slice auth login linux-dev codex before sending prompts to agents in that slice/)
})

test("formatSessionRuntimeStatus uses projected activity over stale legacy prompt state", () => {
  const session = makeSession({
    active_prompt: { id: "prompt-stale" } as never,
    queued_prompts: [{ id: "queued-stale" } as never],
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: false,
      },
    },
    agents: [makeAgent({
      id: "agent-1",
      agent_ref: "agent-1",
      state: "Working",
      is_processing: true,
      remote_execution: {
        worker_kernel_id: "slice:slice-1",
        worker_machine_id: "worker-machine",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
      },
    })],
  })

  const rendered = formatSessionRuntimeStatus(session)

  assert.match(rendered, /prompts: active=0, queued=0, busy_agents=0/)
  assert.match(rendered, /remote runtime: 1 agent, 1 worker, 1 slice, 0 worker run gaps/)
  assert.match(rendered, /agent runtime:\n  - agent-1: Idle /)
  assert.doesNotMatch(rendered, /agent runtime:\n  - agent-1: Working /)
  assert.doesNotMatch(rendered, /next: run \/kernel remote-runtime/)
})

test("formatSessionRuntimeStatus renders projected busy activity over stale idle agent state", () => {
  const session = makeSession({
    prompt_states: {
      "agent-1": {
        active_prompt: null,
        queued_prompts: [],
      },
    },
    agent_activity: {
      "agent-1": {
        status: "working",
        prompt_status: "running",
        busy: true,
        unread_idle_output: false,
        active_turn: {
          prompt_id: "prompt-1",
          provider_run_id: "run-1",
          prompt_origin: "external",
          status: "running",
          phase: "streaming",
        },
      },
    },
    agents: [makeAgent({
      id: "agent-1",
      agent_ref: "agent-1",
      state: "Idle",
      is_processing: false,
    })],
  })

  const rendered = formatSessionRuntimeStatus(session)

  assert.match(rendered, /prompts: active=1, queued=0, busy_agents=1/)
  assert.match(rendered, /agent runtime:\n  - agent-1: Working /)
})

test("formatSessionRuntimeStatus renders unfocused unread idle output as done", () => {
  const session = makeSession({
    focused_agent_id: "agent-focused",
    agent_activity: {
      "agent-done": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: true,
      },
      "agent-focused": {
        status: "idle",
        prompt_status: "none",
        busy: false,
        unread_idle_output: true,
      },
    },
    agents: [
      makeAgent({
        id: "agent-done",
        agent_ref: "agent-done",
        state: "Idle",
        is_processing: false,
      }),
      makeAgent({
        id: "agent-focused",
        agent_ref: "agent-focused",
        state: "Idle",
        is_processing: false,
      }),
    ],
  })

  const rendered = formatSessionRuntimeStatus(session)

  assert.match(rendered, /agent runtime:\n  - agent-done: Done /)
  assert.match(rendered, /\n  - agent-focused: Idle /)
})
