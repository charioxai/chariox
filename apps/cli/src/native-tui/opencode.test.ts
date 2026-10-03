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
    const h = nativeLaunchHarness(t)
    await runOpenCodeNativeTui([...h.args, ...flags])
    assert.equal(h.reads(), 3)
    assert.equal(h.launches(), 1)
    assert.deepEqual(h.attachedSessions, ["official-session"])
    assert.equal(h.closes(), 1)
  })
}

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
function nativeLaunchHarness(t: TestContext, outcome: "ready" | "ended" | "timeout" = "ready") {
  let reads = 0
  let launches = 0
  let closes = 0
  const attachedSessions: string[] = []
  const agent = { id: "agent-1", session_id: "home-session", provider: "opencode", alias: "A1", extension_grants: [] }
  const run = {
    id: "home-projected-run", session_id: "home-session", agent_instance_id: agent.id,
    provider: "opencode", adapter_key: "opencode", client_interface: "native_tui",
    state: "Running", structured_endpoint: "http://127.0.0.1:1", provider_session_id: null,
  }
  t.mock.method(LocalIpcClient.prototype, "send", async (request: Record<string, unknown>) => {
    if ("CreateSession" in request) return { SessionCreated: { session: {
      id: "home-session", workspace_id: "/fixture", worktree_id: "/fixture", agents: [agent],
    }, agent } }
    if ("AttachToSession" in request) return { SessionAttached: { attachment: { id: "attachment-1" } } }
    if ("MoveAgentToRemote" in request) return { AgentMovedToRemote: { agent } }
    if ("LaunchProviderRun" in request) {
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
    const child = new EventEmitter()
    setImmediate(() => child.emit("exit", 0, null))
    return child
  })
  syncBuiltinESMExports()
  t.after(() => { t.mock.restoreAll(); syncBuiltinESMExports() })
  t.mock.method(process.stderr, "write", () => true)
  return {
    args: ["--kernel-url", "ws://127.0.0.1:1/kernel", "--workspace", "/fixture", "--worktree", "/fixture"],
    attachedSessions, reads: () => reads, launches: () => launches, closes: () => closes,
  }
}
