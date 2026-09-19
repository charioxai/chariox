import assert from "node:assert/strict"
import path from "node:path"
import test from "node:test"

import {
  buildPath1FaultCellInvocation,
  buildPath1FaultFailure,
  buildPath1RustProbeInvocation,
  parseArgs,
  runPath1ContextFaultProbe,
  runPath1FaultCell,
} from "./local-rust-fault-drill-runtime.mjs"
import { PATH1_RUNNER_CONTEXT_SCHEMA } from "../../../../scripts/path1-oss-runner-context-adapter.mjs"
import { buildRoomTakeoverReconnectCargoArgs } from "./room-takeover-reconnect-fault-drill.mjs"

const repoRoot = path.resolve(".")

function context() {
  const runId = "path1-fault-test-001"
  const sessionId = "home-session-fault-001"
  const roomId = "room-fault-001"
  return {
    schema: PATH1_RUNNER_CONTEXT_SCHEMA,
    runId,
    allocationId: "allocation-fault-001",
    machineId: "machine-fault-001",
    homeKernelId: "home-kernel-fault-001",
    homeRelayRealmId: "relay-realm-fault-001",
    repoRoot,
    sourceHead: "a".repeat(40),
    sessionId,
    roomId,
    kernelEndpoint: "ws://127.0.0.1:43119/kernel",
    relayEndpoint: "ws://127.0.0.1:43120/relay",
    webEndpoint: "http://127.0.0.1:4321",
    journal: { ref: "journal-fault-001", sequence: 4 },
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
    action: "capability-drills",
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
    official: true,
    kernelAuthoritative: true,
    relayTransportOnly: true,
    capabilities: { reconnect: { recovered: true, persistence: true, history: true } },
    receiptId: "receipt-fault-001",
  }
}

function rustProbeOutput() {
  return `CHARIOX_ROOM_TAKEOVER_RECONNECT_PROBE:${JSON.stringify({
    schema: "chariox.room_takeover_reconnect_probe.v1",
    responseLostAfterCommit: true,
    replayedResponseMatched: true,
    humanOwnershipRetained: true,
    agentMutationBlocked: true,
    takeoverAppliedExactlyOnce: true,
    explicitReleaseRequired: true,
    agentMutationAdmittedAfterRelease: true,
    cleanupComplete: true,
    takeoverEventCount: 1,
  })}`
}

test("standalone Rust fault runtime remains parse-compatible and context-free", () => {
  const options = parseArgs(["--dry-run"], { name: "fault-drill", description: "fault" })
  assert.equal(options.dryRun, true)
  assert.equal(options.runnerContext, null)
})

test("Path 1 fault invocation is explicitly probe-only and cannot launch Cargo authority", () => {
  const invocation = buildPath1FaultCellInvocation(context())
  assert.equal(invocation.createKernel, false)
  assert.equal(invocation.createSession, false)
  assert.equal(invocation.createRelay, false)
  assert.equal(invocation.env.CHARIOX_RELAY_TOKEN, undefined)
  assert.ok(invocation.args.includes("--path1-runner-context"))
})

test("Rust context probe is exact, strict, and authority-free", () => {
  const current = context()
  const invocation = buildPath1RustProbeInvocation(current)
  assert.equal(invocation.command, "cargo")
  assert.deepEqual(invocation.args, buildRoomTakeoverReconnectCargoArgs())
  assert.equal(invocation.cwd, current.repoRoot)
  assert.equal(invocation.createKernel, false)
  assert.equal(invocation.createSession, false)
  assert.equal(invocation.createRelay, false)
  assert.equal(invocation.sameRoomRequired, true)
  assert.equal(invocation.env.CARGO_BUILD_JOBS, "1")
  assert.equal(invocation.env.CHARIOX_PATH1_CONTEXT_MODE, "required-existing-room")
  assert.equal(invocation.env.CHARIOX_RELAY_TOKEN, undefined)
  assert.equal(invocation.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN, undefined)
  assert.equal(invocation.env.PROVIDER_API_KEY, undefined)
  const serializedContext = JSON.parse(invocation.env.CHARIOX_PATH1_RUNNER_CONTEXT_JSON)
  assert.equal(serializedContext.runId, current.runId)
  assert.equal(serializedContext.sessionId, current.sessionId)
  assert.equal(serializedContext.roomId, current.roomId)
  assert.equal(serializedContext.kernelEndpoint, current.kernelEndpoint)
  assert.equal(serializedContext.runtimeBindings.runtimeDigest, current.runtimeBindings.runtimeDigest)
})

test("Rust context probe returns only sanitized typed evidence and preserves the shared binding", async () => {
  const current = context()
  const calls = []
  const result = await runPath1ContextFaultProbe({
    context: current,
    processRunner: {
      async run(invocation) {
        calls.push(invocation)
        return { code: 0, signal: null, stdout: rustProbeOutput(), stderr: "provider output must not escape" }
      },
    },
  })
  assert.equal(calls.length, 1)
  assert.equal(result.status, "passed")
  assert.equal(result.liveObserved, true)
  assert.equal(result.runId, current.runId)
  assert.equal(result.sessionId, current.sessionId)
  assert.equal(result.roomId, current.roomId)
  assert.equal(result.sameRoom, true)
  assert.equal(result.noSecondAuthority, true)
  assert.equal(result.capabilities.takeover.agentMutationBlocked, true)
  assert.equal(result.capabilities.reconnect.replayedResponseMatched, true)
  assert.equal(JSON.stringify(result).includes("provider output"), false)
})

test("missing, stale, or mismatched context fails before an official process can launch", async () => {
  let launches = 0
  const processRunner = { async run() { launches += 1; throw new Error("must not launch") } }
  await assert.rejects(
    () => runPath1ContextFaultProbe({ context: null, processRunner }),
    /runnerContext/u,
  )
  await assert.rejects(
    () => runPath1ContextFaultProbe({ context: { ...context(), kernelEndpoint: "ws://127.0.0.1:43119/kernel?stale=1" }, processRunner }),
    /query material|credential/u,
  )
  const mismatched = context()
  mismatched.clientBindings.web = { ...mismatched.clientBindings.web, sessionId: "stale-session-fault-001" }
  await assert.rejects(
    () => runPath1ContextFaultProbe({ context: mismatched, processRunner }),
    /does not bind to the supplied Room\/session|stale/u,
  )
  assert.equal(launches, 0)
})

test("probe failure is redacted while typed failure evidence preserves the stale-context branch", async () => {
  const current = context()
  await assert.rejects(
    () => runPath1ContextFaultProbe({
      context: current,
      processRunner: { async run() { return { code: 1, signal: null, stdout: "", stderr: "provider secret token=do-not-print" } } },
    }),
    (error) => error instanceof Error && error.message === "official Rust takeover/reconnect probe failed" && !error.message.includes("do-not-print"),
  )
  const failure = buildPath1FaultFailure(current, "stale-context")
  assert.deepEqual(failure.failure, { code: "stale-context", reason: "official Rust takeover/reconnect probe failed" })
  assert.equal(JSON.stringify(failure).includes("do-not-print"), false)
})

test("injected fault evidence is bound to the same Room and missing context-capable probes fail closed", async () => {
  const current = context()
  const events = []
  const result = await runPath1FaultCell({
    context: current,
    clientFactory: factory(events),
    processRunner: {
      async run(invocation) {
        assert.equal(invocation.createSession, false)
        return evidence(current)
      },
    },
  })
  assert.equal(result.sessionId, current.sessionId)
  assert.equal(result.roomId, current.roomId)
  assert.equal(result.noSecondAuthority, true)
  assert.deepEqual(events.filter((event) => event.type === "attach").map((event) => event.mode), ["web", "localTui", "remoteTui"])
  await assert.rejects(
    () => runPath1FaultCell({ context: current, clientFactory: factory([]) }),
    /injected context-capable official probe/u,
  )
  await assert.rejects(
    () => runPath1FaultCell({ context: { ...current, sessionId: "stale-session-fault-001" }, clientFactory: factory([]), processRunner: { async run() { throw new Error("must not launch") } } }),
    /does not bind to the supplied Room\/session|stale/u,
  )
})
