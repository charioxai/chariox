import assert from "node:assert/strict"
import test from "node:test"
import childProcess from "node:child_process"
import { EventEmitter } from "node:events"
import { syncBuiltinESMExports } from "node:module"

import { LocalIpcClient } from "../ipc.js"
import { runCodexNativeTui } from "./codex.js"

for (const [placement, flags] of [
  ["local", []], ["home-worker", ["--machine", "selected-worker"]], ["slice", ["--slice", "selected-slice"]],
] as const) {
  test(`MP-08/MP-11 Codex ${placement} kernel server uses the provider lifecycle and observes launch failure`, async (t) => {
    let launches = 0
    let reads = 0
    let closes = 0
    const agent = { id: "agent-1", session_id: "home-session", provider: "codex", model: "fixture", extension_grants: [] }
    const run = { id: "owned-run", session_id: "home-session", state: "Starting", structured_endpoint: null }
    t.mock.method(LocalIpcClient.prototype, "send", async (request: Record<string, unknown>) => {
      if ("CreateSession" in request) return { SessionCreated: { session: {
        id: "home-session", workspace_id: "/fixture", worktree_id: "/fixture", agents: [agent],
      }, agent } }
      if ("AttachToSession" in request) return { SessionAttached: { attachment: { id: "attachment-1" } } }
      if ("LaunchProviderRun" in request) {
        launches += 1
        assert.equal((request.LaunchProviderRun as Record<string, unknown>).native_tui, true)
        return { ProviderRunLaunchAccepted: { provider_run: run } }
      }
      if ("GetProviderRun" in request) {
        reads += 1
        return { ProviderRun: { provider_run: { ...run, state: "Ended" } } }
      }
      throw new Error(`unexpected kernel request: ${Object.keys(request).join(",")}`)
    })
    t.mock.method(LocalIpcClient.prototype, "close", async () => { closes += 1 })
    await assert.rejects(runCodexNativeTui([
      "--kernel-url", "ws://127.0.0.1:1/kernel", "--workspace", "/fixture", "--worktree", "/fixture",
      "--server-in-kernel", ...flags,
    ]), /Codex provider run ended before attach was ready/)
    assert.equal(launches, 1)
    assert.equal(reads, 1)
    assert.equal(closes, 1)
  })
}

test("Codex --machine runs the model the machine chose, with no usable Codex at home", async (t) => {
  // The worker resolved the model on its own catalog; home serves no Codex models.
  const agent = { id: "agent-1", session_id: "home-session", provider: "codex", model: "worker-model", extension_grants: [] }
  const run = { id: "owned-run", session_id: "home-session", state: "Starting", structured_endpoint: null }
  const created: Record<string, unknown>[] = []
  const launchedModels: unknown[] = []
  t.mock.method(LocalIpcClient.prototype, "send", async (request: Record<string, unknown>) => {
    if ("GetProviderCatalog" in request) return { ProviderCatalog: { catalog: { all: [], default: {}, connected: [] } } }
    if ("CreateSession" in request) {
      created.push(request.CreateSession as Record<string, unknown>)
      return { SessionCreated: { session: {
        id: "home-session", workspace_id: "/fixture", worktree_id: "/fixture", agents: [agent],
      }, agent } }
    }
    if ("AttachToSession" in request) return { SessionAttached: { attachment: { id: "attachment-1" } } }
    if ("LaunchProviderRun" in request) {
      launchedModels.push((request.LaunchProviderRun as Record<string, unknown>).model)
      return { ProviderRunLaunchAccepted: { provider_run: run } }
    }
    if ("GetProviderRun" in request) return { ProviderRun: { provider_run: { ...run, state: "Ended" } } }
    throw new Error(`unexpected kernel request: ${Object.keys(request).join(",")}`)
  })
  t.mock.method(LocalIpcClient.prototype, "close", async () => {})
  await assert.rejects(runCodexNativeTui([
    "--kernel-url", "ws://127.0.0.1:1/kernel", "--workspace", "/fixture", "--worktree", "/fixture",
    "--server-in-kernel", "--machine", "selected-worker",
  ]), /Codex provider run ended before attach was ready/)
  assert.equal(created[0]!.kernel_ref, "selected-worker")
  assert.equal((created[0]!.agent_defaults as { model: string | null }).model, null)
  assert.deepEqual(launchedModels, ["worker-model"])
})

for (const state of ["Running", "Parked"]) {
  test(`MP-08/MP-11 Codex attaches to a ${state} kernel server before its lazy thread exists`, async (t) => {
    const agent = { id: "agent-1", session_id: "home-session", provider: "codex", model: "fixture", extension_grants: [] }
    const run = { id: "owned-run", session_id: "home-session", state: "Starting", structured_endpoint: null, provider_session_id: null }
    let reads = 0
    t.mock.method(LocalIpcClient.prototype, "send", async (request: Record<string, unknown>) => {
      if ("CreateSession" in request) return { SessionCreated: { session: {
        id: "home-session", workspace_id: "/fixture", worktree_id: "/fixture", agents: [agent],
      }, agent } }
      if ("AttachToSession" in request) return { SessionAttached: { attachment: { id: "attachment-1" } } }
      if ("LaunchProviderRun" in request) return { ProviderRunLaunchAccepted: { provider_run: run } }
      if ("GetProviderRun" in request) {
        reads += 1
        return { ProviderRun: { provider_run: { ...run, state, structured_endpoint: "ws://127.0.0.1:1" } } }
      }
      if ("PollRuntimeNotices" in request) return { RuntimeNotices: { notices: [] } }
      throw new Error(`unexpected kernel request: ${Object.keys(request).join(",")}`)
    })
    t.mock.method(LocalIpcClient.prototype, "close", async () => {})
    t.mock.method(process.stderr, "write", () => true)
    t.mock.method(childProcess, "spawn", (_command: string, args: string[]) => {
      assert.equal(reads, 1)
      assert.equal(args[0], "--remote")
      const child = new EventEmitter()
      setImmediate(() => child.emit("exit", 0, null))
      return child
    })
    syncBuiltinESMExports()
    t.after(() => { t.mock.restoreAll(); syncBuiltinESMExports() })
    await runCodexNativeTui([
      "--kernel-url", "ws://127.0.0.1:1/kernel", "--workspace", "/fixture", "--worktree", "/fixture", "--server-in-kernel",
    ])
  })
}
