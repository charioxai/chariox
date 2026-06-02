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
  assert.match(harness.notices.at(-1) ?? "", /auth_status=ready codex, claude/)
  assert.match(harness.notices.at(-1) ?? "", /providers=codex,claude auth_status=ready codex, claude auth=codex:work \(acct-1\),claude:user@example.com\/org=Team\/plan=pro/)
  assert.equal(harness.footers.at(-1)?.message, "listed 1 slice")
})

test("slice command list renders provider account recovery hints", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        providers: ["codex"],
        provider_auth: [],
      }),
    ],
  })

  await handleSliceSlashCommand(harness.deps, command("list"))

  assert.match(harness.notices.at(-1) ?? "", /auth_status=missing codex/)
  assert.match(harness.notices.at(-1) ?? "", /providers=codex auth_status=missing codex auth=-/)
  assert.match(harness.notices.at(-1) ?? "", /next=import or login provider accounts for codex with \/slice auth import linux-dev codex or \/slice auth login linux-dev codex/)
})

test("slice command list keeps provider placeholder for multi-provider recovery hints", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        providers: ["codex", "opencode:openai"],
        provider_auth: [],
      }),
    ],
  })

  await handleSliceSlashCommand(harness.deps, command("list"))

  assert.match(harness.notices.at(-1) ?? "", /auth_status=missing codex, opencode:openai/)
  assert.match(harness.notices.at(-1) ?? "", /providers=codex,opencode:openai auth_status=missing codex, opencode:openai auth=-/)
  assert.match(harness.notices.at(-1) ?? "", /next=import or login provider accounts for codex,opencode:openai with \/slice auth import linux-dev <provider> or \/slice auth login linux-dev <provider>/)
})

test("slice command list renders concrete or placeholder stale-auth recovery hints", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-a",
        providers: ["codex"],
        provider_auth: [{ provider: "codex", state: "not_configured" }],
      }),
      slice({
        id: "slice-2",
        name: "linux-b",
        providers: ["codex", "opencode:openai"],
        provider_auth: [
          { provider: "codex", state: "not_configured" },
          { provider: "opencode:openai", state: "unknown" },
        ],
      }),
    ],
  })

  await handleSliceSlashCommand(harness.deps, command("list"))

  const notice = harness.notices.at(-1) ?? ""
  assert.match(notice, /linux-a[\s\S]*auth_status=refresh codex/)
  assert.match(notice, /linux-b[\s\S]*auth_status=refresh codex, opencode:openai/)
  assert.match(notice, /linux-a[\s\S]*next=refresh provider login for codex with \/slice auth login linux-a codex/)
  assert.match(notice, /linux-b[\s\S]*next=refresh provider login for codex,opencode:openai with \/slice auth login linux-b <provider>/)
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
  assert.match(harness.notices.at(-1) ?? "", /next: inspect slice logs and \/slice audit/)
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
  assert.match(harness.notices.at(-1) ?? "", /next: check relay connectivity/)
  assert.equal(harness.footers.at(-1)?.tone, "error")
})

test("slice command doctor flags missing provider accounts", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        status: "running",
        worker_kernel_id: "kernel-slice",
        relay_endpoint: { url: "wss://relay.example/slice", private: false },
        worktree_id: "/repo/wt",
        providers: ["codex"],
        provider_auth: [],
      }),
    ],
  })

  await handleSliceSlashCommand(harness.deps, command("doctor", "linux-dev"))

  assert.match(harness.notices.at(-1) ?? "", /ok provider CLIs: codex/)
  assert.match(harness.notices.at(-1) ?? "", /fail provider accounts: none/)
  assert.match(harness.notices.at(-1) ?? "", /next: import or login provider accounts for codex/)
  assert.equal(harness.footers.at(-1)?.tone, "error")
})

test("slice command doctor requires provider auth for every advertised provider", async () => {
  const harness = sliceHarness({
    slices: [
      slice({
        id: "slice-1",
        name: "linux-dev",
        status: "running",
        worker_kernel_id: "kernel-slice",
        relay_endpoint: { url: "wss://relay.example/slice", private: false },
        worktree_id: "/repo/wt",
        providers: ["codex", "opencode:openai"],
        provider_auth: [
          { provider: "codex", state: "authenticated", email: "codex@example.com", source: "test" },
        ],
      }),
    ],
  })

  await handleSliceSlashCommand(harness.deps, command("doctor", "linux-dev"))

  assert.match(harness.notices.at(-1) ?? "", /ok provider CLIs: codex,opencode:openai/)
  assert.match(harness.notices.at(-1) ?? "", /fail provider accounts: codex:codex@example.com/)
  assert.match(harness.notices.at(-1) ?? "", /next: import or login provider accounts for opencode:openai with \/slice auth import linux-dev opencode:openai or \/slice auth login linux-dev opencode:openai/)
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

test("slice command audit resolves focused slice and passes limit", async () => {
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

  await handleSliceSlashCommand(harness.deps, command("audit", "--limit", "5"))

  assert.deepEqual(harness.auditRequests, [{ sliceRef: "linux-dev", limit: 5 }])
  assert.match(harness.notices.at(-1) ?? "", /2026-01-02T03:04:05.000Z auth\.import completed slice=linux-dev provider=codex/)
  assert.match(harness.notices.at(-1) ?? "", /status=running display=headless worktree=\/repo\/wt agents=1 worker=kernel-slice/)
  assert.equal(harness.footers.at(-1)?.message, "slice audit linux-dev")
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

test("slice command auth import and remove explain unsupported worker operations", async () => {
  const harness = sliceHarness({
    importedAuthStatus: "not_implemented",
    removedAuthStatus: "not_implemented",
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
  await handleSliceSlashCommand(harness.deps, command("auth", "remove", "codex"))

  assert.equal(harness.footers.at(-2)?.tone, "error")
  assert.match(
    harness.footers.at(-2)?.message ?? "",
    /slice auth import codex is unavailable on this kernel\. Next action: use \/slice auth login linux-dev codex, open \/slice screen linux-dev to configure the account inside the slice, or update\/restart the worker kernel if auth import should be available\./,
  )
  assert.equal(harness.footers.at(-1)?.tone, "error")
  assert.match(
    harness.footers.at(-1)?.message ?? "",
    /slice auth remove codex is unavailable on this kernel\. Next action: open \/slice screen linux-dev to remove the provider account inside the slice, or update\/restart the worker kernel if auth removal should be available\./,
  )
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

test("slice command stop blocks slices with attached agents", async () => {
  const harness = sliceHarness({
    slices: [slice({
      id: "slice-1",
      name: "linux-dev",
      agent_ids: ["agent-build", "agent-review"],
    })],
  })

  await handleSliceSlashCommand(harness.deps, command("stop", "linux-dev"))

  assert.deepEqual(harness.stoppedSlices, [])
  assert.equal(harness.footers.at(-1)?.tone, "error")
  assert.equal(harness.footers.at(-1)?.message, "cannot stop slice linux-dev; move or end attached agents first: agent-build,agent-review")
})

test("slice command delete blocks slices with attached agents", async () => {
  const harness = sliceHarness({
    slices: [slice({
      id: "slice-1",
      name: "linux-dev",
      agent_ids: ["agent-build"],
    })],
  })

  await handleSliceSlashCommand(harness.deps, command("delete", "slice-1"))

  assert.deepEqual(harness.deletedSlices, [])
  assert.equal(harness.footers.at(-1)?.tone, "error")
  assert.equal(harness.footers.at(-1)?.message, "cannot delete slice linux-dev; move or end attached agents first: agent-build")
})

function command(...args: string[]) {
  return { kind: "slice" as const, args, raw: `/slice ${args.join(" ")}` }
}

function sliceHarness(options: {
  readonly slices?: SliceRecord[]
  readonly focusedAgent?: Partial<AgentInstance>
  readonly endpoint?: SliceDisplayEndpoint
  readonly importedAuthStatus?: string
  readonly removedAuthStatus?: string
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
  const stoppedSlices: string[] = []
  const deletedSlices: string[] = []
  const logRequests: Array<{ sliceRef: string; tailLines: number | null | undefined }> = []
  const auditRequests: Array<{ sliceRef: string; limit: number | null | undefined }> = []
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
    stopSlice: async (sliceRef) => {
      stoppedSlices.push(sliceRef)
      return slice({ id: sliceRef, name: sliceRef, status: "stopped" })
    },
    deleteSlice: async (sliceRef) => {
      deletedSlices.push(sliceRef)
      return slice({ id: sliceRef, name: sliceRef })
    },
    importSliceProviderAuth: async (sliceRef, provider) => {
      importedAuth.push({ sliceRef, provider })
      return { slice: slice({ id: sliceRef, name: sliceRef }), provider, status: options.importedAuthStatus ?? "imported" }
    },
    removeSliceProviderAuth: async (sliceRef, provider) => {
      removedAuth.push({ sliceRef, provider })
      return { slice: slice({ id: sliceRef, name: sliceRef }), provider, status: options.removedAuthStatus ?? "removed" }
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
    listSliceAudit: async (sliceRef, limit) => {
      auditRequests.push({ sliceRef, limit })
      return [{
        sequence: 1,
        event_id: "state_evt_1",
        kind: "slice.audit",
        subject_id: "slice-1",
        timestamp_ms: Date.parse("2026-01-02T03:04:05.000Z"),
        payload: {
          slice_id: "slice-1",
          slice_name: sliceRef,
          action: "auth.import",
          outcome: "completed",
          provider: "codex",
          status: "running",
          display_mode: "headless",
          worktree_id: "/repo/wt",
          agent_ids: ["agent-1"],
          worker_kernel_id: "kernel-slice",
        },
      }]
    },
  }
  return { deps, notices, footers, createdSlices, displayEndpointRefs, openedUrls, importedAuth, removedAuth, startedAuthLogins, aliasedAuth, stoppedSlices, deletedSlices, logRequests, auditRequests }
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
