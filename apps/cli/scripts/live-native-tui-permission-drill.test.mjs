import assert from "node:assert/strict"
import path from "node:path"
import test from "node:test"

import {
  buildPath1PermissionCellInvocation,
  parseArgs,
  runPath1PermissionCell,
} from "./live-native-tui-permission-drill.mjs"
import { PATH1_RUNNER_CONTEXT_SCHEMA } from "../../../scripts/path1-oss-runner-context-adapter.mjs"

const repoRoot = path.resolve(".")

function context(provider = "claude") {
  const runId = "path1-permission-test-001"
  const sessionId = "home-session-permission-001"
  const roomId = "room-permission-001"
  return {
    schema: PATH1_RUNNER_CONTEXT_SCHEMA,
    runId,
    allocationId: "allocation-permission-001",
    machineId: "machine-permission-001",
    homeKernelId: "home-kernel-permission-001",
    homeRelayRealmId: "relay-realm-permission-001",
    repoRoot,
    sourceHead: "a".repeat(40),
    sessionId,
    roomId,
    kernelEndpoint: "ws://127.0.0.1:43119/kernel",
    relayEndpoint: "ws://127.0.0.1:43120/relay",
    webEndpoint: "http://127.0.0.1:4321",
    journal: { ref: "journal-permission-001", sequence: 2 },
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

function factory(events) {
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

function evidence(current) {
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
    provider: current.provider,
    official: true,
    kernelAuthoritative: true,
    relayTransportOnly: true,
    actorId: "actor-permission-001",
    threadId: "thread-permission-001",
    computerFallback: { completed: true, permission: true },
    permission: { requested: true, interacted: true, resolved: true },
    receiptId: "receipt-permission-001",
  }
}

test("standalone provider and permission defaults remain unchanged", () => {
  const options = parseArgs([])
  assert.deepEqual(options.providers, ["codex", "opencode"])
  assert.equal(options.runnerContext, null)
})

test("permission Path 1 invocation cannot create a second kernel, session, or relay", () => {
  const current = context()
  const invocation = buildPath1PermissionCellInvocation(current, current.provider)
  assert.equal(invocation.createKernel, false)
  assert.equal(invocation.createSession, false)
  assert.equal(invocation.createRelay, false)
  assert.equal(invocation.sessionId, current.sessionId)
  assert.equal(invocation.roomId, current.roomId)
  assert.equal(invocation.env.CHARIOX_RELAY_TOKEN, undefined)
  assert.ok(invocation.args.includes("--path1-runner-context"))
})

test("injected permission interaction remains on the supplied Room and stale context fails before process launch", async () => {
  const current = context()
  const events = []
  const result = await runPath1PermissionCell({
    context: current,
    provider: current.provider,
    clientFactory: factory(events),
    processRunner: {
      async run(invocation) {
        assert.equal(invocation.sessionId, current.sessionId)
        assert.equal(invocation.createSession, false)
        return evidence(current)
      },
    },
  })
  assert.equal(result.permission.resolved, true)
  assert.equal(result.noSecondAuthority, true)
  assert.deepEqual(events.filter((event) => event.type === "attach").map((event) => event.mode), ["web", "localTui", "remoteTui"])
  let launched = false
  await assert.rejects(
    () => runPath1PermissionCell({
      context: { ...current, roomId: "other-room-001" },
      provider: current.provider,
      clientFactory: factory([]),
      processRunner: { async run() { launched = true; return evidence(current) } },
    }),
    /does not bind to the supplied Room\/session|stale/u,
  )
  assert.equal(launched, false)
})
