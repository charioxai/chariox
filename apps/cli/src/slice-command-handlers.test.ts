import assert from "node:assert/strict"
import test from "node:test"

import type { AgentInstance, SliceDisplayEndpoint, SliceRecord } from "./cli-types.js"
import { handleSliceSlashCommand, type SliceCommandHandlerDeps } from "./slice-command-handlers.js"

test("slice command list renders lifecycle scope and provider auth details", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        status: "running",
        display_mode: "headed",
        worktree_id: "/repo/feature",
        session_ids: ["session-1", "session-2"],
        agent_ids: ["agent-1"],
        relay_endpoint: { url: "wss://relay.example/slice", private: false },
        providers: ["codex", "claude"],
        provider_auth: [
          { provider: "codex", state: "configured", alias: "work", account_id: "acct-1", source: "test" },
          { provider: "claude", state: "authenticated", email: "user@example.com", organization_name: "Team", subscription_type: "pro", source: "test" },
        ],
      }),
    ],
  })

  await handleSliceSlashCommand(harness.deps, command("list"))

  assert.match(harness.notices.at(-1) ?? "", /linux-dev id=slice-1 status=running display=headed/)
  assert.match(harness.notices.at(-1) ?? "", /worktree=\/repo\/feature agents=1 sessions=2/)
  assert.match(harness.notices.at(-1) ?? "", /worker=kernel-slice relay=shared:wss:\/\/relay.example\/slice/)
  assert.match(harness.notices.at(-1) ?? "", /providers=codex,claude auth=codex:work \(acct-1\),claude:user@example.com\/org=Team\/plan=pro/)
  assert.equal(harness.footers.at(-1)?.message, "listed 1 slice")
})

test("slice command create passes display mode and current worktree mount", async () => {
  const harness = sliceHarness()

  await handleSliceSlashCommand(harness.deps, command("create", "qa", "--headed"))

  assert.deepEqual(harness.createdSlices, [{
    name: "qa",
    displayMode: "headed",
    workspaceId: "/repo",
    worktreeId: "/repo/wt",
    workspaceMount: "/repo/wt",
    workerKernelRef: null,
    displayUrl: null,
  }])
  assert.equal(harness.footers.at(-1)?.message, "created slice qa")
})

test("slice command screen resolves focused agent slice and opens endpoint", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
      }),
    ],
    focusedAgent: {
      remote_execution: {
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        execution_lease_id: "lease-1",
        leased_agent_id: "worker-agent",
      },
    },
    endpoint: { slice_id: "slice-1", kind: "novnc", url: "http://127.0.0.1:6080", access: "local" },
  })

  await handleSliceSlashCommand(harness.deps, command("screen"))

  assert.deepEqual(harness.openedUrls, ["http://127.0.0.1:6080"])
  assert.deepEqual(harness.displayEndpointRefs, ["linux-dev"])
  assert.equal(harness.footers.at(-1)?.message, "opened http://127.0.0.1:6080")
})

test("slice command focused lookup prefers explicit agent bindings", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "wrong-by-worker",
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        agent_ids: ["agent-other"],
      }),
      slice({
        id: "slice-2",
        name: "right-by-agent",
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        agent_ids: ["agent-1"],
      }),
    ],
    focusedAgent: {
      remote_execution: {
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        execution_lease_id: "lease-1",
        leased_agent_id: "worker-agent",
      },
    },
    endpoint: { slice_id: "slice-2", kind: "novnc", url: "http://127.0.0.1:6081", access: "local" },
  })

  await handleSliceSlashCommand(harness.deps, command("screen"))

  assert.deepEqual(harness.displayEndpointRefs, ["right-by-agent"])
  assert.deepEqual(harness.openedUrls, ["http://127.0.0.1:6081"])
})

test("slice command doctor renders health checks", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        status: "unhealthy",
        display_mode: "headed",
        worker_kernel_id: null,
        worktree_id: "/repo/wt",
        relay_endpoint: null,
        session_ids: ["session-1"],
        agent_ids: ["agent-1"],
      }),
    ],
  })

  await handleSliceSlashCommand(harness.deps, command("doctor", "linux-dev"))

  assert.match(harness.notices.at(-1) ?? "", /slice doctor linux-dev \(slice-1\)/)
  assert.match(harness.notices.at(-1) ?? "", /fail lifecycle: unhealthy/)
  assert.match(harness.notices.at(-1) ?? "", /fail display: headed/)
  assert.match(harness.notices.at(-1) ?? "", /ok relay: none/)
  assert.match(harness.notices.at(-1) ?? "", /ok agents: 1 attached/)
  assert.equal(harness.footers.at(-1)?.tone, "error")
})

test("slice command doctor flags running slices without a relay endpoint", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        status: "running",
        worker_kernel_id: "kernel-slice",
        relay_endpoint: null,
      }),
    ],
  })

  await handleSliceSlashCommand(harness.deps, command("doctor", "linux-dev"))

  assert.match(harness.notices.at(-1) ?? "", /fail relay: none/)
  assert.equal(harness.footers.at(-1)?.tone, "error")
})

test("slice command logs renders focused slice diagnostics", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
      }),
    ],
    focusedAgent: {
      remote_execution: {
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        execution_lease_id: "lease-1",
        leased_agent_id: "worker-agent",
      },
    },
  })

  await handleSliceSlashCommand(harness.deps, command("logs", "--tail", "25"))

  assert.deepEqual(harness.logRequests, [{ sliceRef: "linux-dev", tailLines: 25 }])
  assert.match(harness.notices.at(-1) ?? "", /slice logs linux-dev \(slice-1\)/)
  assert.match(harness.notices.at(-1) ?? "", /== provision path=\/tmp\/slice.log truncated ==/)
  assert.match(harness.notices.at(-1) ?? "", /slice booted/)
  assert.equal(harness.footers.at(-1)?.message, "slice logs linux-dev")
})

test("slice command auth import can target the focused agent slice", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
      }),
    ],
    focusedAgent: {
      remote_execution: {
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        execution_lease_id: "lease-1",
        leased_agent_id: "worker-agent",
      },
    },
  })

  await handleSliceSlashCommand(harness.deps, command("auth", "import", "codex"))

  assert.deepEqual(harness.importedAuth, [{ sliceRef: "linux-dev", provider: "codex" }])
  assert.equal(harness.footers.at(-1)?.message, "slice auth import codex: imported")
})

test("slice command auth remove can target the focused agent slice", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
      }),
    ],
    focusedAgent: {
      remote_execution: {
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        execution_lease_id: "lease-1",
        leased_agent_id: "worker-agent",
      },
    },
  })

  await handleSliceSlashCommand(harness.deps, command("auth", "remove", "opencode"))

  assert.deepEqual(harness.removedAuth, [{ sliceRef: "linux-dev", provider: "opencode" }])
  assert.equal(harness.footers.at(-1)?.message, "slice auth remove opencode: removed")
})

test("slice command auth login starts provider login in focused agent slice", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
      }),
    ],
    focusedAgent: {
      remote_execution: {
        worker_kernel_id: "kernel-slice",
        worker_machine_id: "machine-slice",
        execution_lease_id: "lease-1",
        leased_agent_id: "worker-agent",
      },
    },
  })

  await handleSliceSlashCommand(harness.deps, command("auth", "login", "codex"))

  assert.deepEqual(harness.startedAuthLogins, [{ sliceRef: "linux-dev", provider: "codex" }])
  assert.match(harness.notices.at(-1) ?? "", /url=https:\/\/auth.example/)
  assert.match(harness.notices.at(-1) ?? "", /code=ABCD-EFGH/)
  assert.equal(harness.footers.at(-1)?.message, "slice auth login codex: started")
})

test("slice command auth alias sets and clears provider aliases", async () => {
  const harness = sliceHarness()

  await handleSliceSlashCommand(harness.deps, command("auth", "alias", "slice-1", "codex", "work", "account"))
  await handleSliceSlashCommand(harness.deps, command("auth", "alias", "slice-1", "codex", "clear"))

  assert.deepEqual(harness.aliasedAuth, [
    { sliceRef: "slice-1", provider: "codex", alias: "work account" },
    { sliceRef: "slice-1", provider: "codex", alias: null },
  ])
  assert.equal(harness.footers.at(-2)?.message, "slice auth alias codex: work account")
  assert.equal(harness.footers.at(-1)?.message, "slice auth alias codex: cleared")
})

function command(...args: string[]) {
  return { kind: "slice" as const, args, raw: `/slice ${args.join(" ")}` }
}

function sliceHarness(options: {
  readonly slices?: SliceRecord[]
  readonly focusedAgent?: Partial<AgentInstance>
  readonly endpoint?: SliceDisplayEndpoint
} = {}) {
  const notices: string[] = []
  const footers: Array<{ message: string; tone: "info" | "error" }> = []
  const createdSlices: unknown[] = []
  const displayEndpointRefs: string[] = []
  const openedUrls: string[] = []
  const importedAuth: Array<{ sliceRef: string; provider: string }> = []
  const removedAuth: Array<{ sliceRef: string; provider: string }> = []
  const startedAuthLogins: Array<{ sliceRef: string; provider: string }> = []
  const aliasedAuth: Array<{ sliceRef: string; provider: string; alias: string | null }> = []
  const logRequests: Array<{ sliceRef: string; tailLines: number | null | undefined }> = []
  const slices = options.slices ?? []
  const endpoint = options.endpoint ?? { slice_id: "slice-1", kind: "novnc", url: "http://slice.local", access: "local" }
  const focusedAgent = agent(options.focusedAgent)
  const deps: SliceCommandHandlerDeps = {
    currentWorkspaceTarget: () => "/repo",
    currentWorktreeTarget: () => "/repo/wt",
    focusedAgentId: () => focusedAgent.id,
    resolveSessionAgent: () => ({ agent: focusedAgent, error: null }),
    flashFooter: (message, tone) => { footers.push({ message, tone }) },
    appendNotice: (message) => { notices.push(message) },
    openExternalUrl: async (url) => {
      openedUrls.push(url)
      return true
    },
    listSlices: async () => slices,
    createSlice: async (createOptions) => {
      createdSlices.push(createOptions)
      return slice({
        id: "slice-created",
        name: createOptions.name,
        ...(createOptions.displayMode ? { display_mode: createOptions.displayMode } : {}),
      })
    },
    getSlice: async (sliceRef) => slices.find((entry) => entry.id === sliceRef || entry.name === sliceRef) ?? slice({ id: sliceRef, name: sliceRef }),
    startSlice: async (sliceRef) => slice({ id: sliceRef, name: sliceRef, status: "running" }),
    stopSlice: async (sliceRef) => slice({ id: sliceRef, name: sliceRef, status: "stopped" }),
    deleteSlice: async (sliceRef) => slice({ id: sliceRef, name: sliceRef }),
    importSliceProviderAuth: async (sliceRef, provider) => {
      importedAuth.push({ sliceRef, provider })
      return { slice: slice({ id: sliceRef, name: sliceRef }), provider, status: "imported" }
    },
    removeSliceProviderAuth: async (sliceRef, provider) => {
      removedAuth.push({ sliceRef, provider })
      return { slice: slice({ id: sliceRef, name: sliceRef }), provider, status: "removed" }
    },
    startSliceProviderLogin: async (sliceRef, provider) => {
      startedAuthLogins.push({ sliceRef, provider })
      return {
        slice: slice({ id: sliceRef, name: sliceRef }),
        login: {
          provider,
          login_kind: "device",
          verification_url: "https://auth.example",
          user_code: "ABCD-EFGH",
          status: "started",
          message: "Open https://auth.example and enter ABCD-EFGH",
        },
      }
    },
    setSliceProviderAuthAlias: async (sliceRef, provider, alias) => {
      aliasedAuth.push({ sliceRef, provider, alias })
      return { slice: slice({ id: sliceRef, name: sliceRef }), provider, alias }
    },
    getSliceDisplayEndpoint: async (sliceRef) => {
      displayEndpointRefs.push(sliceRef)
      return endpoint
    },
    getSliceLogs: async (sliceRef, tailLines) => {
      logRequests.push({ sliceRef, tailLines })
      return {
        slice: slice({ id: "slice-1", name: sliceRef }),
        entries: [{
          source: "provision",
          path: "/tmp/slice.log",
          text: "slice booted",
          truncated: true,
        }],
      }
    },
  }
  return { deps, notices, footers, createdSlices, displayEndpointRefs, openedUrls, importedAuth, removedAuth, startedAuthLogins, aliasedAuth, logRequests }
}

function slice(overrides: Partial<SliceRecord> = {}): SliceRecord {
  return {
    id: "slice-1",
    name: "slice-1",
    owner_kernel_id: "kernel-local",
    owner_machine_id: "machine-local",
    backend: "local_docker",
    os: "linux",
    status: "running",
    workspace_mount: null,
    workspace_id: null,
    worktree_id: null,
    session_ids: [],
    agent_ids: [],
    display_mode: "headless",
    worker_kernel_ref: "slice:slice-1",
    worker_kernel_id: "kernel-slice",
    worker_machine_id: "machine-slice",
    relay_endpoint: null,
    providers: [],
    provider_auth: [],
    display_endpoint: null,
    created_at_ms: 0,
    updated_at_ms: 0,
    ...overrides,
  }
}

function agent(overrides: Partial<AgentInstance> = {}): AgentInstance {
  return {
    id: "agent-1",
    agent_ref: "agent-1",
    session_id: "session-1",
    alias: null,
    provider: "codex",
    model: "codex/gpt-5",
    effort: "high",
    worktree_id: "/repo/wt",
    state: "Idle",
    is_processing: false,
    grid_row: 0,
    grid_col: 0,
    grid_row_span: 1,
    grid_col_span: 1,
    created_at_ms: 0,
    last_activity_at_ms: 0,
    ...overrides,
  }
}
