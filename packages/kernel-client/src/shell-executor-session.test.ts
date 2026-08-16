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

test("executeShellCommand removes shell-local variables", async () => {
  const context = createDefaultShellContext({
    workspace: "/repo",
    worktree: "/repo",
    variables: { stale: "agent-1", keep: "session-1" },
  })
  const result = await executeShellCommand(parseShellCommand("unset stale"), context, { client: fakeClient(() => ({})).client })
  assert.equal(result.ok, true)
  assert.deepEqual(result.variableRemovals, ["stale"])
  const next = applyShellCommandResult(context, result)
  assert.deepEqual(next.variables, { keep: "session-1" })
})

test("executeShellCommand creates a session and binds assignment", async () => {
  const session = makeSession({ id: "session-2", worktree_id: "/repo/qa", focused_agent_id: "agent-1" })
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { CreateSession: { workspace_id: "/repo", worktree_id: "/repo/qa", alias: null, slice_ref: null } })
    return { SessionCreated: { session } }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --dir qa as s"), context, {
    client: fake.client,
  })
  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /workspace live sync: config default/)
  assert.deepEqual(result.bindings, { s: "session-2" })
  assert.deepEqual(result.contextUpdates, {
    sessionId: "session-2",
    agentId: "agent-1",
    workspace: "/repo",
    worktree: "/repo/qa",
    workspaceId: "/repo",
    worktreeId: "/repo/qa",
  })
})

test("executeShellCommand rejects deprecated metaagent session creation", async () => {
  const fake = fakeClient(() => {
    throw new Error("kernel should not be called for deprecated metaagent session creation")
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --meta --dir qa as s"), context, {
    client: fake.client,
  })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /creating separate metaagents is deprecated/)
  assert.equal(fake.requests.length, 0)
})

test("executeShellCommand rejects deprecated metaagent sessions before slice handling", async () => {
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const fake = fakeClient(() => {
    throw new Error("kernel should not be called for invalid metaagent slice placement")
  })

  const result = await executeShellCommand(parseShellCommand("session new --meta --slice slice-1"), context, {
    client: fake.client,
  })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /creating separate metaagents is deprecated/)
  assert.equal(fake.requests.length, 0)
})

test("executeShellCommand does not adopt stale focused agent ids from session payloads", async () => {
  const session = makeSession({
    id: "session-2",
    worktree_id: "/repo/qa",
    focused_agent_id: "stale-agent",
    agents: [makeAgent({ id: "agent-1" })],
  })
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { CreateSession: { workspace_id: "/repo", worktree_id: "/repo/qa", alias: null, slice_ref: null } })
    return { SessionCreated: { session } }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })

  const result = await executeShellCommand(parseShellCommand("session new --dir qa"), context, {
    client: fake.client,
  })

  assert.equal(result.ok, true)
  assert.deepEqual(result.contextUpdates, {
    sessionId: "session-2",
    agentId: undefined,
    workspace: "/repo",
    worktree: "/repo/qa",
    workspaceId: "/repo",
    worktreeId: "/repo/qa",
  })
})

test("executeShellCommand reports explicit session workspace live sync mode after create", async () => {
  const session = makeSession({
    id: "session-2",
    worktree_id: "/repo/qa",
    focused_agent_id: "agent-1",
    workspace_live_sync_mode: "tracked",
  })
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { CreateSession: { workspace_id: "/repo", worktree_id: "/repo/qa", alias: null, slice_ref: null } })
    return { SessionCreated: { session } }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --dir qa"), context, {
    client: fake.client,
  })

  assert.equal(result.ok, true)
  assert.match(result.message ?? "", /workspace live sync: tracked \(selected workspace\/worktree only; other repositories unrestricted\); use `workspace sync off` to disable/)
})

test("executeShellCommand creates and starts a headless slice for a new session", async () => {
  const session = makeSession({ id: "session-2", worktree_id: "/repo/qa", focused_agent_id: "agent-1" })
  const requests: Record<string, unknown>[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("CreateSlice" in request) {
      return { SliceCreated: { slice: { id: "slice-1" } } }
    }
    if ("StartSlice" in request) {
      return { SliceStarted: { slice: { id: "slice-1" } } }
    }
    if ("CreateSession" in request) {
      return { SessionCreated: { session } }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --dir qa --slice new as s"), context, {
    client: fake.client,
  })

  assert.equal(result.ok, true)
  assert.deepEqual(requests.map((request) => Object.keys(request)[0]), ["CreateSlice", "StartSlice", "CreateSession"])
  const createRequest = requests[0] as { CreateSlice: { name: string } }
  assert.match(createRequest.CreateSlice.name, /^qa-slice-/)
  assert.deepEqual({
    ...requests[0],
    CreateSlice: {
      ...(requests[0] as { CreateSlice: Record<string, unknown> }).CreateSlice,
      name: "<dynamic>",
    },
  }, {
    CreateSlice: {
      name: "<dynamic>",
      backend: "local_docker",
      os: "linux",
      display_mode: "headless",
      workspace_id: "/repo",
      worktree_id: "/repo/qa",
      workspace_mount: "/repo/qa",
      worker_kernel_ref: null,
      display_url: null,
      provider_auth: [],
      from_saved_state: null,
      base: null,
    },
  })
  assert.deepEqual(requests[1], { StartSlice: { slice_ref: "slice-1" } })
  assert.deepEqual(requests[2], { CreateSession: { workspace_id: "/repo", worktree_id: "/repo/qa", alias: null, slice_ref: "slice-1" } })
  assert.deepEqual(result.bindings, { s: "session-2" })
})

test("executeShellCommand creates and starts a headed slice for a new session", async () => {
  const session = makeSession({ id: "session-2", worktree_id: "/repo/qa", focused_agent_id: "agent-1" })
  const requests: Record<string, unknown>[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("CreateSlice" in request) {
      return { SliceCreated: { slice: { id: "slice-1" } } }
    }
    if ("StartSlice" in request) {
      return { SliceStarted: { slice: { id: "slice-1" } } }
    }
    if ("CreateSession" in request) {
      return { SessionCreated: { session } }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --dir qa --slice new:headed as s"), context, {
    client: fake.client,
  })

  assert.equal(result.ok, true)
  assert.equal((requests[0] as { CreateSlice: { display_mode: string } }).CreateSlice.display_mode, "headed")
  assert.deepEqual(requests.map((request) => Object.keys(request)[0]), ["CreateSlice", "StartSlice", "CreateSession"])
})

test("executeShellCommand session new usage advertises inline slice display modes", async () => {
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --kernel worker-1"), context, {
    client: fakeClient(() => {
      throw new Error("unexpected request")
    }).client,
  })

  assert.equal(result.ok, false)
  assert.match(result.message ?? "", /--slice off\|new:headless\|new:headed\|<slice-ref>/)
  assert.doesNotMatch(result.message ?? "", /--slice-display/)
})

test("executeShellCommand reuses only slices scoped to the session worktree", async () => {
  const session = makeSession({ id: "session-2", worktree_id: "/repo/qa", focused_agent_id: "agent-1" })
  const requests: Record<string, unknown>[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("ListSlices" in request) {
      return {
        SlicesListed: {
          slices: [{
            id: "slice-qa",
            name: "qa",
            status: "stopped",
            workspace_id: "/repo",
            worktree_id: "/repo/qa",
          }],
        },
      }
    }
    if ("StartSlice" in request) {
      return { SliceStarted: { slice: { id: "slice-qa" } } }
    }
    if ("CreateSession" in request) {
      return { SessionCreated: { session } }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  const result = await executeShellCommand(parseShellCommand("session new --dir qa --slice qa"), context, {
    client: fake.client,
  })

  assert.equal(result.ok, true)
  assert.deepEqual(requests.map((request) => Object.keys(request)[0]), ["ListSlices", "StartSlice", "CreateSession"])
  assert.deepEqual(requests[1], { StartSlice: { slice_ref: "slice-qa" } })
  assert.deepEqual(requests[2], { CreateSession: { workspace_id: "/repo", worktree_id: "/repo/qa", alias: null, slice_ref: "slice-qa" } })
})

test("executeShellCommand rejects slices from another worktree", async () => {
  const fake = fakeClient((request) => {
    assert.deepEqual(request, { ListSlices: null })
    return {
      SlicesListed: {
        slices: [{
          id: "slice-main",
          name: "main",
          status: "running",
          workspace_id: "/repo",
          worktree_id: "/repo/main",
        }],
      },
    }
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo" })
  await assert.rejects(
    () => executeShellCommand(parseShellCommand("session new --dir qa --slice main"), context, {
      client: fake.client,
    }),
    /slice main is scoped to worktree \/repo\/main, not \/repo\/qa/,
  )
})

test("executeShellCommand creates manually managed slices scoped to the current worktree", async () => {
  const requests: Record<string, unknown>[] = []
  const fake = fakeClient((request) => {
    requests.push(request)
    if ("CreateSlice" in request) {
      return {
        SliceCreated: {
          slice: {
            id: "slice-manual",
            name: "linux-a",
            backend: "local_docker",
            os: "linux",
            status: "stopped",
            display_mode: "headed",
            workspace_id: "/repo",
            worktree_id: "/repo/feature",
            workspace_mount: "/repo/feature",
            worker_kernel_ref: null,
            worker_kernel_id: null,
            worker_machine_id: null,
            providers: [],
            session_ids: [],
            agent_ids: [],
            provider_auth: [],
            created_at_ms: 0,
            updated_at_ms: 0,
          },
        },
      }
    }
    throw new Error(`unexpected request ${JSON.stringify(request)}`)
  })
  const context = createDefaultShellContext({ workspace: "/repo", worktree: "/repo/feature" })
  const result = await executeShellCommand(parseShellCommand("slice create linux-a --headed as sl"), context, { client: fake.client })

  assert.equal(result.ok, true)
  assert.deepEqual(requests, [{
    CreateSlice: {
      name: "linux-a",
      backend: "local_docker",
      os: "linux",
      display_mode: "headed",
      workspace_id: "/repo",
      worktree_id: "/repo/feature",
      workspace_mount: "/repo/feature",
      worker_kernel_ref: null,
      display_url: null,
      provider_auth: [],
      from_saved_state: null,
      base: null,
    },
  }])
  assert.deepEqual(result.bindings, { sl: "slice-manual" })
})
