import assert from "node:assert/strict"
import test from "node:test"

import type {
  AgentInstance,
  RuntimeProviderRun,
  RuntimeSession,
  SliceRecord,
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
      owner_user_id: "alice",
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
    slices: [slice()],
    worktree: "/repo/worktrees/feature",
    run: providerRun("session-run-1"),
    grantedMcps: ["filesystem"],
    grantedSkills: ["review"],
    providerLines: ["  proxy:          ws://127.0.0.1:1234"],
    promptPolicy: "native prompts pass through",
  })

  assert.match(banner, /^\[arroba codex native-tui\]/)
  assert.match(banner, /arroba session: session-1 \(Review\)/)
  assert.match(banner, /arroba agent:   A1 \(builder\) \[id=agent-1\]/)
  assert.match(banner, /home kernel:    home-kernel@home-machine/)
  assert.match(banner, /session owner:  alice/)
  assert.match(banner, /worktree:       \/repo\/worktrees\/feature/)
  assert.match(banner, /placement:      slice linux-dev \(worker=hetzner, kernel=worker-kernel, lease=lease-1, leased_agent=leased-agent-1, active_run=worker-run-1\)/)
  assert.match(banner, /slice:          linux-dev \(id=slice-1, status=running, display=headed, worktree=\/repo\/worktrees\/feature, agents=1\)/)
  assert.match(banner, /slice auth:     ready codex/)
  assert.match(banner, /slice accounts: codex=work \(work@example.com\)/)
  assert.doesNotMatch(banner, /slice next:/)
  assert.match(banner, /live sync:      tracked \(selected workspace\/worktree only; other repositories unrestricted\)/)
  assert.match(banner, /extensions:     mcp=filesystem \(active tools home-proxy\); skill=review \(skills snapshot\)/)
  assert.match(banner, /ext runtime:    home-proxy tools execute on home with home-owned grants and credentials; skills are passive snapshots/)
  assert.match(banner, /ext boundary:   home validates every call; credentials never leave home/)
  assert.match(banner, /remote ext sync: pending/)
  assert.match(banner, /ext sync next:  home keeps stale home-proxy calls blocked until the worker manifest settles; run \/extension sync-status A1; run \/machine kernels hetzner if it does not settle; use \/extension sync-retry A1 after worker connectivity is healthy/)
  assert.match(banner, /provider run:   session-run-1/)
  assert.match(banner, /proxy:          ws:\/\/127\.0\.0\.1:1234/)
  assert.match(banner, /prompt policy:  native prompts pass through/)
})

test("remote native TUI banner keeps MCPs and skills worker-local while scripts stay home-proxied", () => {
  const banner = formatNativeTuiRuntimeBanner({
    surface: "opencode native-tui",
    session: session({ workspace_live_sync_mode: "tracked" }),
    agent: agent({
      remote_execution: {
        worker_kernel_id: "worker-kernel",
        worker_machine_id: "hetzner",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
      },
      extension_grants: [
        { kind: "mcp", name: "filesystem" },
        { kind: "skill", name: "review" },
        { kind: "script", name: "release" },
      ],
    }),
    worktree: "/repo",
    run: providerRun("native-run-1", { client_interface: "native_tui" }),
    grantedMcps: ["filesystem"],
    grantedSkills: ["review"],
  })

  assert.match(banner, /extensions:     mcp=filesystem \(worker-local, hash-validated\); skill=review \(worker-local, hash-validated\)/)
  assert.match(banner, /ext runtime:    native MCP and skill grants use hash-validated worker-local installs; scripts and connectors remain home-proxied/)
  assert.match(banner, /ext boundary:   worker validates native MCP and skill hashes; home validates script and connector calls without exporting credentials/)
  assert.match(banner, /remote ext sync: pending/)
})

test("remote native TUI banner does not invent manifest sync for worker-local MCPs and skills", () => {
  const banner = formatNativeTuiRuntimeBanner({
    surface: "codex native-tui",
    session: session({ workspace_live_sync_mode: "tracked" }),
    agent: agent({
      remote_execution: {
        worker_kernel_id: "worker-kernel",
        worker_machine_id: "hetzner",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
      },
      extension_grants: [
        { kind: "mcp", name: "filesystem" },
        { kind: "skill", name: "review" },
      ],
    }),
    worktree: "/repo",
    run: providerRun("native-run-1", { client_interface: "native_tui" }),
    grantedMcps: ["filesystem"],
    grantedSkills: ["review"],
  })

  assert.match(banner, /ext runtime:    native MCP and skill grants use hash-validated worker-local installs/)
  assert.match(banner, /ext boundary:   worker validates native MCP and skill hashes; provider credentials and execution stay on the worker/)
  assert.doesNotMatch(banner, /remote ext sync:/)
})

test("native TUI runtime banner calls out missing slice provider auth", () => {
  const banner = formatNativeTuiRuntimeBanner({
    surface: "opencode native-tui",
    session: session({ workspace_live_sync_mode: "tracked" }),
    agent: agent({
      remote_execution: {
        worker_kernel_id: "worker-kernel",
        worker_machine_id: "hetzner",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
      },
    }),
    slices: [slice({
      provider_auth: [],
      providers: ["codex", "opencode:openai"],
    })],
    worktree: "/repo/worktrees/feature",
  })

  assert.match(banner, /placement:      slice linux-dev/)
  assert.match(banner, /slice auth:     missing codex, opencode:openai/)
  assert.match(banner, /slice accounts: none/)
  assert.match(banner, /slice next:     run \/slice doctor linux-dev; configure missing provider accounts codex, opencode:openai with \/slice auth import linux-dev codex or \/slice auth login linux-dev codex/)
})

test("native TUI runtime banner reports stopped slice lifecycle recovery", () => {
  const banner = formatNativeTuiRuntimeBanner({
    surface: "codex native-tui",
    session: session({ workspace_live_sync_mode: "tracked" }),
    agent: agent({
      remote_execution: {
        worker_kernel_id: "worker-kernel",
        worker_machine_id: "hetzner",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
      },
    }),
    slices: [slice({ status: "stopped" })],
    worktree: "/repo/worktrees/feature",
  })

  assert.match(banner, /slice:          linux-dev \(id=slice-1, status=stopped/)
  assert.match(banner, /slice next:     run \/slice start linux-dev; if it does not become running, run \/slice doctor linux-dev and \/slice logs linux-dev/)
})

test("native TUI runtime banner reports unhealthy slice lifecycle recovery", () => {
  const banner = formatNativeTuiRuntimeBanner({
    surface: "opencode native-tui",
    session: session({ workspace_live_sync_mode: "managed" }),
    agent: agent({
      remote_execution: {
        worker_kernel_id: "worker-kernel",
        worker_machine_id: "hetzner",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
      },
    }),
    slices: [slice({ status: "unhealthy" })],
    worktree: "/repo/worktrees/feature",
  })

  assert.match(banner, /slice:          linux-dev \(id=slice-1, status=unhealthy/)
  assert.match(banner, /slice next:     run \/slice doctor linux-dev; inspect \/slice logs linux-dev and \/slice audit linux-dev; then try \/slice start linux-dev or recreate the slice/)
})

test("native TUI runtime banner can show explicit slice lookup failures", () => {
  const banner = formatNativeTuiRuntimeBanner({
    surface: "codex native-tui",
    session: session({ workspace_live_sync_mode: "tracked" }),
    agent: agent({
      remote_execution: {
        worker_kernel_id: "slice:linux-dev",
        worker_machine_id: "hetzner",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
      },
    }),
    worktree: "/repo",
    sliceLookupError: "kernel did not return slice inventory",
  })

  assert.match(banner, /placement:      remote \(worker=hetzner, kernel=slice:linux-dev, lease=lease-1, leased_agent=leased-agent-1\)/)
  assert.match(banner, /slice lookup:   kernel did not return slice inventory/)
  assert.match(banner, /slice next:     run \/slice list; run \/slice doctor linux-dev if listed; run \/kernel remote-runtime if slice inventory stays unavailable/)
})

test("native TUI runtime banner renders local defaults without inventing state", () => {
  const banner = formatNativeTuiRuntimeBanner({
    surface: "opencode native-tui",
    session: session({ workspace_live_sync_mode: null }),
    agent: agent({ remote_execution: null }),
    worktree: "",
  })

  assert.match(banner, /home kernel:    -/)
  assert.doesNotMatch(banner, /session owner:/)
  assert.match(banner, /worktree:       \/repo/)
  assert.match(banner, /placement:      worker-local/)
  assert.match(banner, /live sync:      config default/)
  assert.match(banner, /extensions:     none/)
  assert.match(banner, /ext runtime:    worker-local: definition and execution stay on the selected kernel/)
  assert.match(banner, /ext boundary:   definition and execution stay on the selected kernel/)
  assert.doesNotMatch(banner, /remote ext sync:/)
  assert.doesNotMatch(banner, /provider run:/)
})

test("native TUI runtime banner surfaces blocked remote extension sync", () => {
  const banner = formatNativeTuiRuntimeBanner({
    surface: "claude native-tui",
    session: session({ workspace_live_sync_mode: "managed" }),
    agent: agent({
      remote_execution: {
        worker_kernel_id: "worker-kernel",
        worker_machine_id: "hetzner",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
      },
      extension_grants: [{ kind: "mcp", name: "filesystem" }],
      remote_extension_manifest_sync: {
        state: "failed",
        pending_revoke: true,
        manifest_hash: "abcdef1234567890",
        last_error: "worker offline",
      },
    }),
    worktree: "/repo",
    grantedMcps: ["filesystem"],
  })

  assert.match(banner, /extensions:     mcp=filesystem \(active tools home-proxy\)/)
  assert.match(banner, /ext runtime:    home-proxy: home owns definition, grants, credentials, and execution; worker receives projected tools/)
  assert.match(banner, /ext boundary:   home validates every call; credentials never leave home/)
  assert.match(banner, /remote ext sync: failed, pending revoke, hash=abcdef123456, error=worker offline/)
  assert.match(banner, /ext sync next:  keep the home revoke in place; run \/extension sync-status A1; run \/machine kernels hetzner if the revoke stays pending; use \/extension sync-retry A1 after the worker reconnects/)
})

test("native TUI runtime banner keeps final revokes visible after grants are gone", () => {
  const banner = formatNativeTuiRuntimeBanner({
    surface: "codex native-tui",
    session: session({ workspace_live_sync_mode: "managed" }),
    agent: agent({
      remote_execution: {
        worker_kernel_id: "worker-kernel",
        worker_machine_id: "hetzner",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
      },
      extension_grants: [],
      remote_extension_manifest_sync: {
        state: "failed",
        pending_revoke: true,
        manifest_hash: "empty-hash",
        last_error: "worker offline",
      },
    }),
    worktree: "/repo",
  })

  assert.match(banner, /extensions:     none \(final revoke pending\)/)
  assert.match(banner, /remote ext sync: failed, pending revoke, hash=empty-hash, error=worker offline/)
  assert.match(banner, /ext sync next:  keep the home revoke in place; run \/extension sync-status A1; run \/machine kernels hetzner if the revoke stays pending; use \/extension sync-retry A1 after the worker reconnects/)
})

test("native TUI runtime banner surfaces provider run recovery actions", () => {
  const mismatched = formatNativeTuiRuntimeBanner({
    surface: "codex native-tui",
    session: session({ workspace_live_sync_mode: "tracked" }),
    agent: agent({}),
    worktree: "/repo",
    run: providerRun("session-run-1", { agent_instance_id: null }),
  })
  assert.match(mismatched, /provider run:   session-run-1/)
  assert.match(mismatched, /provider next:  run \/kernel health and \/provider processes; export a debug bundle, then close or relaunch the mismatched provider run before sending more prompts to A1/)

  const missingWorkerRun = formatNativeTuiRuntimeBanner({
    surface: "opencode native-tui",
    session: session({ workspace_live_sync_mode: "managed" }),
    agent: agent({
      state: "Working",
      is_processing: true,
      remote_execution: {
        worker_kernel_id: "worker-kernel",
        worker_machine_id: "hetzner",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
      },
    }),
    worktree: "/repo",
    run: providerRun("session-run-2"),
  })
  assert.match(missingWorkerRun, /provider next:  run \/kernel remote-runtime and \/machine kernels hetzner; reconnect or relaunch the remote\/slice worker before sending prompts to that remote\/slice agent if no active worker run appears/)
})

test("native TUI runtime banner omits sync line for passive skill snapshots", () => {
  const banner = formatNativeTuiRuntimeBanner({
    surface: "opencode native-tui",
    session: session({ workspace_live_sync_mode: "tracked" }),
    agent: agent({
      remote_execution: {
        worker_kernel_id: "worker-kernel",
        worker_machine_id: "hetzner",
        execution_lease_id: "lease-1",
        leased_agent_id: "leased-agent-1",
      },
      extension_grants: [{ kind: "skill", name: "review" }],
    }),
    worktree: "/repo",
    grantedSkills: ["review"],
  })

  assert.match(banner, /extensions:     skill=review \(skills snapshot\)/)
  assert.match(banner, /ext runtime:    skill snapshot: home projects passive content; executable helpers require a separate tool grant/)
  assert.match(banner, /ext boundary:   passive content only; executable helpers require separate tool grants/)
  assert.doesNotMatch(banner, /remote ext sync:/)
})

function session(overrides: Partial<RuntimeSession>): RuntimeSession {
  return {
    id: "session-1",
    project_id: "project-default",
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

function providerRun(id: string, overrides: Partial<RuntimeProviderRun> = {}): RuntimeProviderRun {
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
    ...overrides,
  }
}

function slice(overrides: Partial<SliceRecord> = {}): SliceRecord {
  return {
    id: "slice-1",
    name: "linux-dev",
    owner_kernel_id: "home-kernel",
    owner_machine_id: "home-machine",
    backend: "local_docker",
    os: "linux",
    status: "running",
    workspace_id: "/repo",
    worktree_id: "/repo/worktrees/feature",
    agent_ids: ["agent-1"],
    display_mode: "headed",
    worker_kernel_ref: "slice:slice-1",
    worker_kernel_id: "worker-kernel",
    worker_machine_id: "hetzner",
    provider_auth: [{
      provider: "codex",
      state: "authenticated",
      alias: "work",
      email: "work@example.com",
      source: "slice",
    }],
    created_at_ms: 1,
    updated_at_ms: 1,
    ...overrides,
  }
}
