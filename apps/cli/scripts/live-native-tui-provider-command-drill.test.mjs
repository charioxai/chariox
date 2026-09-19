import assert from "node:assert/strict"
import path from "node:path"
import test from "node:test"

import {
  buildPath1ProviderCellInvocation,
  parseArgs,
  readPath1RunnerContext,
  runPath1ProviderCell,
} from "./live-native-tui-provider-command-drill.mjs"
import { PATH1_RUNNER_CONTEXT_SCHEMA } from "../../../scripts/path1-oss-runner-context-adapter.mjs"

const repoRoot = path.resolve(".")

function context(provider = "codex") {
  const runId = "path1-provider-command-test-001"
  const sessionId = "home-session-provider-001"
  const roomId = "room-provider-001"
  return {
    schema: PATH1_RUNNER_CONTEXT_SCHEMA,
    runId,
    allocationId: "allocation-provider-001",
    machineId: "machine-provider-001",
    homeKernelId: "home-kernel-provider-001",
    homeRelayRealmId: "relay-realm-provider-001",
    repoRoot,
    sourceHead: "a".repeat(40),
    sessionId,
    roomId,
    kernelEndpoint: "ws://127.0.0.1:43119/kernel",
    relayEndpoint: "ws://127.0.0.1:43120/relay",
    webEndpoint: "http://127.0.0.1:4321",
    journal: { ref: "journal-provider-001", sequence: 1 },
    runtimeBindings: {
      cloudRevision: "b".repeat(40),
      ossRevision: "a".repeat(40),
      runtimeDigest: `sha256:${"c".repeat(64)}`,
      localDaemonProtocol: 333,
      relayPeerProtocol: 55,
    },
    surfaceManifest: { sameRoom: true, acceptsRunnerContext: true, normalKernelAuthority: true, relayTransportOnly: true },
    clientModes: { web: true, localTui: true, remoteTui: true },
    clientBindings: Object.fromEntries(["web", "localTui", "remoteTui"].map((mode) => [mode, {
      runId, sessionId, roomId, authority: "home-kernel", attached: true,
    }])),
    authority: { kernel: "normal-kernel", relay: "transport-only", room: "kernel-owned" },
    action: "provider-cell",
    provider,
  }
}

function clients(events) {
  return {
    async attach(value) {
      events.push({ type: "attach", ...value })
      return { binding: { runId: value.runId, sessionId: value.sessionId, roomId: value.roomId, authority: "home-kernel" } }
    },
    async verifyRoom(value) {
      events.push({ type: "verify", ...value })
      return { ...value, kernelAuthoritative: true, relayTransportOnly: true }
    },
    async close() { events.push({ type: "close" }) },
  }
}

function evidence(current, provider = current.provider) {
  return {
    schema: "chariox.path1.runner-context-result.v1",
    source: "deployed-oss-live-drill",
    liveObserved: true,
    dryRun: false,
    sourceTestOnly: false,
    runId: current.runId,
    sourceHead: current.sourceHead,
    sessionId: current.sessionId,
    roomId: current.roomId,
    provider,
    official: true,
    kernelAuthoritative: true,
    relayTransportOnly: true,
    actorId: "actor-provider-001",
    threadId: "thread-provider-001",
    computerFallback: { completed: true, sameRoom: true },
    receiptId: "receipt-provider-001",
  }
}

test("manual defaults remain standalone and context-free", () => {
  const options = parseArgs([])
  assert.deepEqual(options.providers, ["codex", "opencode"])
  assert.equal(options.runnerContext, null)
  assert.equal(readPath1RunnerContext([], {}), null)
})

test("Path 1 invocation is existing-session-only and carries no credential environment", () => {
  const current = context()
  const invocation = buildPath1ProviderCellInvocation(current, "codex")
  assert.equal(invocation.sessionId, current.sessionId)
  assert.equal(invocation.roomId, current.roomId)
  assert.equal(invocation.createKernel, false)
  assert.equal(invocation.createSession, false)
  assert.equal(invocation.createRelay, false)
  assert.ok(invocation.args.includes("--path1-runner-context"))
  assert.ok(invocation.args.includes("--session") === false)
  assert.equal(invocation.env.CHARIOX_RELAY_TOKEN, undefined)
  assert.equal(invocation.env.PROVIDER_API_KEY, undefined)
})

test("injected provider cell proves one Room, preserves failure, and never leaks child error text", async () => {
  const current = context()
  const events = []
  const result = await runPath1ProviderCell({
    context: current,
    provider: "codex",
    clientFactory: clients(events),
    processRunner: {
      async run(invocation) {
        assert.equal(invocation.createSession, false)
        assert.equal(invocation.sessionId, current.sessionId)
        return evidence(current)
      },
    },
  })
  assert.equal(result.sessionId, current.sessionId)
  assert.equal(result.roomId, current.roomId)
  assert.equal(result.noSecondAuthority, true)
  assert.deepEqual(events.filter((event) => event.type === "attach").map((event) => event.mode), ["web", "localTui", "remoteTui"])
  await assert.rejects(
    () => runPath1ProviderCell({
      context: current,
      provider: "codex",
      clientFactory: clients([]),
      processRunner: { async run() { throw new Error("provider bearer secret must not escape") } },
    }),
    (error) => error.code === "official-failed" && !error.message.includes("bearer"),
  )
  await assert.rejects(
    () => runPath1ProviderCell({
      context: { ...current, sessionId: "stale-session-001" },
      provider: "codex",
      clientFactory: clients([]),
      processRunner: { async run() { throw new Error("must not run") } },
    }),
    /does not bind to the supplied Room\/session|stale/u,
  )
})
