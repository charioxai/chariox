import assert from "node:assert/strict"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import test from "node:test"

import type {
  AgentInstance,
  CharioxMcpServerConfig,
  CharioxSkillMetadata,
  ProviderProcessInfo,
  WorkspaceLinkDefinition,
} from "./kernel-types.js"
import { applyShellCommandResult, createDefaultShellContext, parseShellCommand } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"
import {
  daemonHealth,
  fakeClient,
  makeAgent,
  makeSession,
  makeWorkflow,
  makeWorkflowPublication,
  makeWorkflowRun,
  makeWorkflowWatchdog,
} from "./shell-executor.test-support.js"

test("executeShellCommand renders slice doctor diagnostics", async () => {
  const fake = fakeClient((request) => {
    if ("GetSlice" in request) {
      return {
        Slice: {
          slice: {
            id: "slice-1",
            name: "linux-a",
            owner_kernel_id: "home-kernel",
            owner_machine_id: "home-machine",
            backend: "local_docker",
            os: "linux",
            status: "unhealthy",
            display_mode: "headed",
            workspace_id: "/repo",
            worktree_id: "/repo/feature",
            workspace_mount: "/repo/feature",
            worker_kernel_ref: "slice:linux-a",
            worker_kernel_id: null,
            worker_machine_id: null,
            providers: ["codex"],
            session_ids: ["session-1"],
            agent_ids: ["agent-1"],
            provider_auth: [{
              provider: "codex",
              state: "authenticated",
              alias: "daily",
              email: "dev@example.com",
              organization_name: "Team",
              subscription_type: "pro",
            }],
            relay_endpoint: { url: "wss://relay.example/slice", private: false },
            display_endpoint: null,
            created_at_ms: 0,
            updated_at_ms: 0,
          },
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/feature" })
  const result = await executeShellCommand(parseShellCommand("slice doctor linux-a"), context, { client: fake.client })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /slice doctor linux-a id=slice-1/)
  assert.match(result.message ?? "", /ok owner: home-kernel@home-machine/)
  assert.match(result.message ?? "", /fail lifecycle: unhealthy/)
  assert.match(result.message ?? "", /ok relay: shared:wss:\/\/relay.example\/slice/)
  assert.match(result.message ?? "", /fail display: headed/)
  assert.match(result.message ?? "", /ok agents: 1 attached/)
  assert.match(result.message ?? "", /ok provider CLIs: codex/)
  assert.match(result.message ?? "", /ok provider accounts: codex:daily \(dev@example.com\)\/org=Team\/plan=pro/)
  assert.match(result.message ?? "", /next: inspect slice logs and audit/)
})

test("executeShellCommand does not infer shared slice relay authority", async () => {
  const fake = fakeClient((request) => {
    if ("GetSlice" in request) {
      return {
        Slice: {
          slice: {
            id: "slice-1",
            name: "linux-a",
            owner_kernel_id: "home-kernel",
            owner_machine_id: "home-machine",
            backend: "local_docker",
            os: "linux",
            status: "running",
            display_mode: "headless",
            workspace_id: "/repo",
            worktree_id: "/repo/feature",
            workspace_mount: "/repo/feature",
            worker_kernel_ref: "slice:linux-a",
            worker_kernel_id: "worker-1",
            worker_machine_id: "machine-1",
            providers: ["codex"],
            session_ids: [],
            agent_ids: [],
            provider_auth: [],
            relay_endpoint: { url: "wss://relay.example/slice" },
            display_endpoint: null,
            created_at_ms: 0,
            updated_at_ms: 0,
          },
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/feature" })
  const result = await executeShellCommand(parseShellCommand("slice status linux-a"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /owner=home-kernel@home-machine authority=home-managed/)
  assert.match(result.message ?? "", /relay=unknown:wss:\/\/relay.example\/slice/)
})

test("executeShellCommand renders concrete slice storage recovery", async () => {
  const fake = fakeClient((request) => {
    if ("GetSlice" in request) {
      return {
        Slice: {
          slice: {
            id: "slice-1",
            name: "linux-a",
            owner_kernel_id: "home-kernel",
            owner_machine_id: "home-machine",
            backend: "local_docker",
            os: "linux",
            status: "unhealthy",
            display_mode: "headless",
            workspace_id: "/repo",
            worktree_id: "/repo/feature",
            workspace_mount: "/repo/feature",
            worker_kernel_ref: "slice:linux-a",
            worker_kernel_id: null,
            worker_machine_id: null,
            providers: ["codex"],
            session_ids: [],
            agent_ids: [],
            provider_auth: [],
            relay_endpoint: null,
            display_endpoint: null,
            last_operation: "start",
            last_operation_status: "failed",
            last_error: "slice storage preflight failed for desktop: /home/slice has 0MiB free, needs 256MiB",
            created_at_ms: 0,
            updated_at_ms: 0,
          },
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/feature" })
  const status = await executeShellCommand(parseShellCommand("slice status linux-a"), context, { client: fake.client })
  const doctor = await executeShellCommand(parseShellCommand("slice doctor linux-a"), context, { client: fake.client })

  assert.equal(status.ok, true)
  assert.match(status.message ?? "", /slice storage preflight failed for desktop/)
  assert.match(status.message ?? "", /next=free Docker\/Colima disk or delete unused slice containers\/volumes; then restart slice linux-a or recreate it if startup still fails/)
  assert.doesNotMatch(status.message ?? "", /next=open logs and audit/)
  assert.equal(doctor.ok, false)
  assert.match(doctor.message ?? "", /fail last operation: start:failed error=slice storage preflight failed/)
  assert.match(doctor.message ?? "", /next: free Docker\/Colima disk or delete unused slice containers\/volumes; then restart slice linux-a or recreate it if startup still fails/)
  assert.doesNotMatch(doctor.message ?? "", /inspect slice logs and audit, then retry/)
})

test("executeShellCommand renders slice account recovery hints", async () => {
  const fake = fakeClient((request) => {
    if ("ListSlices" in request) {
      return {
        SlicesListed: {
          slices: [{
            id: "slice-1",
            name: "linux-a",
            owner_kernel_id: "home-kernel",
            owner_machine_id: "home-machine",
            backend: "local_docker",
            os: "linux",
            status: "running",
            display_mode: "headless",
            workspace_id: "/repo",
            worktree_id: "/repo/feature",
            workspace_mount: "/repo/feature",
            worker_kernel_ref: "slice:linux-a",
            worker_kernel_id: "kernel-slice",
            worker_machine_id: "machine-slice",
            providers: ["codex"],
            session_ids: [],
            agent_ids: [],
            provider_auth: [],
            relay_endpoint: { url: "wss://relay.example/slice", private: false },
            display_endpoint: null,
            created_at_ms: 0,
            updated_at_ms: 0,
          }],
        },
      }
    }
    if ("GetSlice" in request) {
      return {
        Slice: {
          slice: {
            id: "slice-1",
            name: "linux-a",
            backend: "local_docker",
            os: "linux",
            status: "running",
            display_mode: "headless",
            workspace_id: "/repo",
            worktree_id: "/repo/feature",
            workspace_mount: "/repo/feature",
            worker_kernel_ref: "slice:linux-a",
            worker_kernel_id: "kernel-slice",
            worker_machine_id: "machine-slice",
            providers: ["codex"],
            session_ids: [],
            agent_ids: [],
            provider_auth: [],
            relay_endpoint: { url: "wss://relay.example/slice", private: false },
            display_endpoint: null,
            created_at_ms: 0,
            updated_at_ms: 0,
          },
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/feature" })
  const result = await executeShellCommand(parseShellCommand("slice list"), context, { client: fake.client })
  const doctor = await executeShellCommand(parseShellCommand("slice doctor linux-a"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /linux-a id=slice-1 status=running/)
  assert.match(result.message ?? "", /owner=home-kernel@home-machine authority=home-managed/)
  assert.match(result.message ?? "", /auth_status=missing codex/)
  assert.match(result.message ?? "", /providers=codex auth_status=missing codex auth=-/)
  assert.match(result.message ?? "", /next=import or login provider accounts for codex with \/slice auth import linux-a codex <account-profile> or \/slice auth login linux-a codex <account-profile>/)
  assert.equal(doctor.ok, false)
  assert.match(doctor.message ?? "", /fail provider accounts: missing codex/)
  assert.match(doctor.message ?? "", /next: import or login provider accounts for codex/)
})

test("executeShellCommand requires slice auth coverage for every advertised provider", async () => {
  const fake = fakeClient((request) => {
    if ("GetSlice" in request) {
      return {
        Slice: {
          slice: {
            id: "slice-1",
            name: "linux-a",
            backend: "local_docker",
            os: "linux",
            status: "running",
            display_mode: "headless",
            workspace_id: "/repo",
            worktree_id: "/repo/feature",
            workspace_mount: "/repo/feature",
            worker_kernel_ref: "slice:linux-a",
            worker_kernel_id: "kernel-slice",
            worker_machine_id: "machine-slice",
            providers: ["codex", "opencode:openai"],
            session_ids: [],
            agent_ids: [],
            provider_auth: [{
              provider: "codex",
              state: "authenticated",
              email: "codex@example.com",
            }],
            relay_endpoint: { url: "wss://relay.example/slice", private: false },
            display_endpoint: null,
            created_at_ms: 0,
            updated_at_ms: 0,
          },
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/feature" })
  const result = await executeShellCommand(parseShellCommand("slice doctor linux-a"), context, { client: fake.client })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /ok provider CLIs: codex,opencode:openai/)
  assert.match(result.message ?? "", /fail provider accounts: codex:codex@example.com; missing opencode:openai/)
  assert.match(result.message ?? "", /next: import or login provider accounts for opencode:openai with \/slice auth import linux-a opencode:openai <account-profile> or \/slice auth login linux-a opencode:openai <account-profile>/)
})

test("executeShellCommand renders concrete slice stale-auth recovery", async () => {
  const baseSlice = {
    id: "slice-1",
    name: "linux-a",
    backend: "local_docker",
    os: "linux",
    status: "running",
    display_mode: "headless",
    workspace_id: "/repo",
    worktree_id: "/repo/feature",
    workspace_mount: "/repo/feature",
    worker_kernel_ref: "slice:linux-a",
    worker_kernel_id: "kernel-slice",
    worker_machine_id: "machine-slice",
    session_ids: [],
    agent_ids: [],
    relay_endpoint: { url: "wss://relay.example/slice", private: false },
    display_endpoint: null,
    created_at_ms: 0,
    updated_at_ms: 0,
  }
  const fake = fakeClient((request) => {
    if ("ListSlices" in request) {
      return {
        SlicesListed: {
          slices: [
            {
              ...baseSlice,
              id: "slice-1",
              name: "linux-a",
              providers: ["codex"],
              provider_auth: [{ provider: "codex", state: "not_configured" }],
            },
            {
              ...baseSlice,
              id: "slice-2",
              name: "linux-b",
              providers: ["codex", "opencode:openai"],
              provider_auth: [
                { provider: "codex", state: "not_configured" },
                { provider: "opencode:openai", state: "unknown" },
              ],
            },
          ],
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/feature" })
  const result = await executeShellCommand(parseShellCommand("slice list"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /linux-a[\s\S]*auth_status=refresh codex/)
  assert.match(result.message ?? "", /linux-b[\s\S]*auth_status=refresh codex, opencode:openai/)
  assert.match(result.message ?? "", /linux-a[\s\S]*next=refresh provider login for codex with \/slice auth login linux-a codex <account-profile>/)
  assert.match(result.message ?? "", /linux-b[\s\S]*next=refresh provider login for codex,opencode:openai with \/slice auth login linux-b codex <account-profile>; for opencode:openai use \/slice auth login linux-b opencode:openai <account-profile>/)
})

test("executeShellCommand treats unsupported slice auth responses as failures", async () => {
  const slice = {
    id: "slice-1",
    name: "linux-a",
    backend: "ssh_docker",
    os: "linux",
    status: "stopped",
    display_mode: "headless",
    workspace_id: "/repo",
    worktree_id: "/repo/feature",
    workspace_mount: "/repo/feature",
    worker_kernel_ref: "slice:linux-a",
    worker_kernel_id: null,
    worker_machine_id: null,
    providers: [],
    session_ids: [],
    agent_ids: [],
    provider_auth: [],
    relay_endpoint: null,
    display_endpoint: null,
    created_at_ms: 0,
    updated_at_ms: 0,
  }
  const fake = fakeClient((request) => {
    if ("ImportSliceProviderAuth" in request) {
      return { SliceProviderAuthImported: { slice, provider: "codex", status: "not_implemented" } }
    }
    if ("RemoveSliceProviderAuth" in request) {
      return { SliceProviderAuthRemoved: { slice, provider: "codex", status: "not_implemented" } }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/feature" })

  const imported = await executeShellCommand(parseShellCommand("slice auth import linux-a codex default"), context, { client: fake.client })
  const removed = await executeShellCommand(parseShellCommand("slice auth remove linux-a codex default"), context, { client: fake.client })

  assert.equal(imported.ok, false)
  assert.match(
    imported.message ?? "",
    /auth import codex is unavailable on this kernel\. Next action: use \/slice auth login linux-a codex <account-profile>, open \/slice screen linux-a to configure the account inside the slice, or update\/restart the worker kernel if auth import should be available\./,
  )
  assert.equal(removed.ok, false)
  assert.match(
    removed.message ?? "",
    /auth remove codex is unavailable on this kernel\. Next action: open \/slice screen linux-a to remove the provider account inside the slice, or update\/restart the worker kernel if auth removal should be available\./,
  )
})

test("executeShellCommand resolves focused agent slice by attached agent id", async () => {
  const requests: Record<string, unknown>[] = []
  const wrongWorkerSlice = {
    id: "slice-wrong",
    name: "wrong-by-worker",
    backend: "local_docker",
    os: "linux",
    status: "running",
    display_mode: "headless",
    workspace_id: "/repo",
    worktree_id: "/repo/other",
    workspace_mount: "/repo/other",
    worker_kernel_ref: "slice:wrong-by-worker",
    worker_kernel_id: "kernel-agent",
    worker_machine_id: "machine-agent",
    providers: [],
    session_ids: ["session-1"],
    agent_ids: ["agent-other"],
    provider_auth: [],
    relay_endpoint: null,
    display_endpoint: null,
    created_at_ms: 0,
    updated_at_ms: 0,
  }
  const slice = {
    id: "slice-1",
    name: "linux-a",
    backend: "local_docker",
    os: "linux",
    status: "running",
    display_mode: "headless",
    workspace_id: "/repo",
    worktree_id: "/repo/feature",
    workspace_mount: "/repo/feature",
    worker_kernel_ref: "slice:linux-a",
    worker_kernel_id: "kernel-slice-other",
    worker_machine_id: "machine-slice-other",
    providers: ["codex"],
    session_ids: ["session-1"],
    agent_ids: ["agent-1"],
    provider_auth: [{
      provider: "codex",
      state: "not_configured",
      auth_type: "oauth",
    }],
    relay_endpoint: { url: "wss://relay.example/slice", private: false },
    display_endpoint: null,
    created_at_ms: 0,
    updated_at_ms: 0,
  }
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListAgents" in request) {
      return { AgentsListed: { agents: [makeAgent({ remote_execution: { worker_kernel_id: "kernel-agent", worker_machine_id: "machine-agent", execution_lease_id: "lease-1", leased_agent_id: "leased-agent-1" } })] } }
    }
    if ("ListSlices" in request) {
      return { SlicesListed: { slices: [wrongWorkerSlice, slice] } }
    }
    if ("GetSlice" in request) {
      return { Slice: { slice } }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo/feature",
    sessionId: "session-1",
    agentId: "agent-1",
  })
  const result = await executeShellCommand(parseShellCommand("slice status"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.deepEqual(requests.map((request) => Object.keys(request)[0]), ["ListAgents", "ListSlices", "GetSlice"])
  assert.deepEqual(requests[2], { GetSlice: { slice_ref: "slice-1" } })
  assert.match(result.message ?? "", /linux-a id=slice-1 status=running/)
  assert.match(result.message ?? "", /relay=shared:wss:\/\/relay.example\/slice/)
  assert.match(result.message ?? "", /auth=codex:oauth/)
  assert.match(result.message ?? "", /next=refresh provider login for codex with \/slice auth login linux-a codex <account-profile>/)
})

test("executeShellCommand renders slice logs", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("GetSliceLogs" in request) {
      return {
        SliceLogs: {
          slice: {
            id: "slice-1",
            name: "linux-a",
            backend: "local_docker",
            os: "linux",
            status: "running",
            display_mode: "headless",
            workspace_id: "/repo",
            worktree_id: "/repo/feature",
            workspace_mount: "/repo/feature",
            worker_kernel_ref: "slice:linux-a",
            worker_kernel_id: "kernel-slice",
            worker_machine_id: "machine-slice",
            providers: [],
            session_ids: [],
            agent_ids: [],
            provider_auth: [],
            display_endpoint: null,
            created_at_ms: 0,
            updated_at_ms: 0,
          },
          entries: [{
            source: "container",
            path: null,
            text: "worker started",
            truncated: false,
          }],
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/feature" })
  const result = await executeShellCommand(parseShellCommand("slice logs linux-a --tail 50"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.deepEqual(requests, [{ GetSliceLogs: { slice_ref: "linux-a", tail_lines: 50 } }])
  assert.match(result.message ?? "", /slice logs linux-a id=slice-1/)
  assert.match(result.message ?? "", /== container ==/)
  assert.match(result.message ?? "", /worker started/)
})

test("executeShellCommand renders slice audit", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListSliceAudit" in request) {
      return {
        SliceAuditListed: {
          events: [
            {
              sequence: 1,
              event_id: "state_evt_1",
              kind: "slice.audit",
              subject_id: "slice-1",
              timestamp_ms: Date.parse("2026-01-02T03:04:05.000Z"),
              payload: {
                slice_id: "slice-1",
                slice_name: "linux-a",
                action: "auth.import",
                outcome: "completed",
                provider: "codex",
                status: "running",
                backend: "local_docker",
                display_mode: "headless",
                worktree_id: "/repo/feature",
                session_ids: ["session-1", "session-2"],
                agent_ids: ["agent-1"],
                worker_kernel_id: "kernel-slice",
                worker_machine_id: "machine-slice",
              },
            },
            {
              sequence: 2,
              event_id: "state_evt_2",
              kind: "slice.audit",
              subject_id: "slice-1",
              timestamp_ms: Date.parse("2026-01-02T03:04:06.000Z"),
              payload: {
                slice_id: "slice-1",
                slice_name: "linux-a",
                action: "auth.login",
                outcome: "failed",
                provider: "opencode",
                message: "login failed",
                status: "running",
                backend: "local_docker",
                display_mode: "headless",
                worktree_id: "/repo/feature",
                session_ids: ["session-1", "session-2"],
                agent_ids: ["agent-1"],
                worker_kernel_id: "kernel-slice",
                worker_machine_id: "machine-slice",
              },
            },
          ],
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/feature" })
  const result = await executeShellCommand(parseShellCommand("slice audit linux-a --limit 5"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.deepEqual(requests, [{ ListSliceAudit: { slice_ref: "linux-a", limit: 5 } }])
  assert.match(result.message ?? "", /2026-01-02T03:04:05.000Z auth\.import completed slice=linux-a provider=codex/)
  assert.match(result.message ?? "", /status=running backend=local_docker display=headless worktree=\/repo\/feature sessions=2 agents=1 worker=kernel-slice machine=machine-slice/)
  assert.match(result.message ?? "", /2026-01-02T03:04:06.000Z auth\.login failed slice=linux-a provider=opencode message=login failed/)
  assert.match(result.message ?? "", /next: run \/slice doctor linux-a; retry with \/slice auth login linux-a opencode <account-profile> or \/slice auth import linux-a opencode <account-profile>/)
})
