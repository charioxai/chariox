import assert from "node:assert/strict"
import childProcess from "node:child_process"
import { EventEmitter } from "node:events"
import { syncBuiltinESMExports } from "node:module"
import test, { type TestContext } from "node:test"

import { LocalIpcClient } from "../ipc.js"
import { parseNativeOpenCodeArgs, runOpenCodeNativeTui } from "./opencode.js"

test("native OpenCode accepts an exact model override", () => {
  const options = parseNativeOpenCodeArgs([
    "--model",
    "opencode/kimi-k2.7-code",
    "--server-in-kernel",
  ])

  assert.equal(options.model, "opencode/kimi-k2.7-code")
  assert.equal(options.serverInKernel, true)
})

test("native OpenCode defaults to the kernel server that installs runtime MCP", () => {
  // An externally started bare `opencode serve` has no Chariox MCP binding.
  // The kernel waits for that absent server and the native launcher stays Starting.
  const options = parseNativeOpenCodeArgs(["--model", "opencode-go/deepseek-v4.1-flash"])

  assert.equal(options.serverInKernel, true)
})

for (const [placement, flags] of [
  ["local", []], ["home-worker", ["--machine", "selected-worker"]], ["slice", ["--slice", "selected-slice"]],
] as const) {
  test(`MP-08/MP-10 OpenCode ${placement} attach waits for the home run readiness projection`, async (t) => {
    const workerDirectory = placement === "local" ? "/fixture" : "/workspace"
    const h = nativeLaunchHarness(t, "ready", workerDirectory)
    await runOpenCodeNativeTui([...h.args, ...flags])
    assert.equal(h.reads(), 3)
    assert.equal(h.launches(), 1)
    assert.deepEqual(h.attachedSessions, ["official-session"])
    assert.deepEqual(h.attachedDirectories, [workerDirectory])
    assert.equal(h.closes(), 1)
  })
}

for (const model of ["openai/gpt-5.2", "opencode-go/kimi-k2.6"]) {
  test(`OpenCode launches the connected upstream model ${model}: the kernel, not the launcher, resolves it`, async (t) => {
    const h = nativeLaunchHarness(t)
    await runOpenCodeNativeTui([...h.args, "--model", model])
    assert.equal((h.createdSessions[0]!.agent_defaults as { model: string }).model, model)
    assert.deepEqual(h.launchedModels, [model])
  })
}

test("OpenCode --machine creates the agent on the machine, with no move from home", async (t) => {
  const h = nativeLaunchHarness(t, "ready", "/workspace")
  await runOpenCodeNativeTui([...h.args, "--machine", "selected-worker", "--model", "openai/gpt-5.2"])
  assert.equal(h.createdSessions[0]!.kernel_ref, "selected-worker")
  assert.deepEqual(h.launchedModels, ["openai/gpt-5.2"])
})

test("MP-08/MP-10 OpenCode attach retains the worktree fallback for older run projections", async (t) => {
  const h = nativeLaunchHarness(t)
  await runOpenCodeNativeTui(h.args)
  assert.deepEqual(h.attachedDirectories, ["/fixture"])
})

test("MP-08/MP-10 OpenCode remote attach fails when the projected run ends before readiness", async (t) => {
  const h = nativeLaunchHarness(t, "ended")
  await assert.rejects(runOpenCodeNativeTui([...h.args, "--machine", "selected-worker"]), /ended before attach was ready/)
  assert.deepEqual(h.attachedSessions, [])
  assert.equal(h.closes(), 1)
})

test("MP-08/MP-10 OpenCode slice attach has bounded readiness failure", async (t) => {
  const h = nativeLaunchHarness(t, "timeout")
  let now = 0
  t.mock.method(Date, "now", () => { const value = now; now += 30_001; return value })
  await assert.rejects(runOpenCodeNativeTui([...h.args, "--slice", "selected-slice"]), /timed out waiting for OpenCode provider run to become ready/)
  assert.deepEqual(h.attachedSessions, [])
  assert.equal(h.closes(), 1)
})

// Exercise the real launcher, request builders, readiness loop and proxy lifecycle.
// Only kernel transport replies and the provider executable are synthetic.
function nativeLaunchHarness(t: TestContext, outcome: "ready" | "ended" | "timeout" = "ready", workingDirectory?: string) {
  let reads = 0
  let launches = 0
  let closes = 0
  const attachedSessions: string[] = []
  const attachedDirectories: string[] = []
  const createdSessions: Record<string, unknown>[] = []
  const launchedModels: unknown[] = []
  const agent = { id: "agent-1", session_id: "home-session", provider: "opencode", alias: "A1", model: "opencode/big-pickle", extension_grants: [] }
  const run = {
    id: "home-projected-run", session_id: "home-session", agent_instance_id: agent.id,
    provider: "opencode", adapter_key: "opencode", client_interface: "native_tui",
    working_directory: workingDirectory,
    state: "Running", structured_endpoint: "http://127.0.0.1:1", provider_session_id: null,
  }
  t.mock.method(LocalIpcClient.prototype, "send", async (request: Record<string, unknown>) => {
    if ("CreateSession" in request) {
      const created = request.CreateSession as Record<string, unknown>
      createdSessions.push(created)
      // The kernel resolves the named model on the execution target; this one serves it.
      const named = (created.agent_defaults as { model?: string } | undefined)?.model
      if (named && named !== "default") agent.model = named
      return { SessionCreated: { session: {
        id: "home-session", workspace_id: "/fixture", worktree_id: "/fixture", agents: [agent],
      }, agent } }
    }
    // Home's catalog: only the free OpenCode models; a connected upstream is its own entry.
    if ("GetProviderCatalog" in request) return { ProviderCatalog: { catalog: {
      all: [{ id: "opencode", name: "OpenCode", models: { "big-pickle": { id: "big-pickle", name: "Big Pickle" } } }],
      default: { opencode: "big-pickle" }, connected: ["opencode"],
    } } }
    if ("AttachToSession" in request) return { SessionAttached: { attachment: { id: "attachment-1" } } }
    if ("LaunchProviderRun" in request) {
      launchedModels.push((request.LaunchProviderRun as Record<string, unknown>).model)
      launches += 1
      return { ProviderRunLaunchAccepted: { provider_run: run } }
    }
    if ("GetProviderRun" in request) {
      assert.deepEqual(request.GetProviderRun, { provider_run_id: run.id })
      reads += 1
      return { ProviderRun: { provider_run: {
        ...run,
        state: outcome === "ended" ? "Ended" : reads === 2 ? "Starting" : "Running",
        provider_session_id: outcome === "ready" && reads >= 2 ? "official-session" : null,
      } } }
    }
    if ("PollRuntimeNotices" in request) return { RuntimeNotices: { notices: [] } }
    throw new Error(`unexpected kernel request: ${Object.keys(request).join(",")}`)
  })
  t.mock.method(LocalIpcClient.prototype, "close", async () => { closes += 1 })
  t.mock.method(childProcess, "spawn", (_command: string, args: string[]) => {
    assert.equal(args[0], "attach")
    assert.equal(reads, 3, "provider attach must follow kernel readiness")
    attachedSessions.push(args[args.indexOf("--session") + 1]!)
    attachedDirectories.push(args[args.indexOf("--dir") + 1]!)
    const child = new EventEmitter()
    setImmediate(() => child.emit("exit", 0, null))
    return child
  })
  syncBuiltinESMExports()
  t.after(() => { t.mock.restoreAll(); syncBuiltinESMExports() })
  t.mock.method(process.stderr, "write", () => true)
  return {
    args: ["--kernel-url", "ws://127.0.0.1:1/kernel", "--workspace", "/fixture", "--worktree", "/fixture"],
    attachedSessions, attachedDirectories, createdSessions, launchedModels, reads: () => reads, launches: () => launches, closes: () => closes,
  }
}
