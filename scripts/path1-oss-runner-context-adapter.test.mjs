import assert from "node:assert/strict"
import path from "node:path"
import test from "node:test"

import {
  PATH1_RUNNER_CONTEXT_CLIENT_MODES,
  PATH1_RUNNER_CONTEXT_PROVIDERS,
  PATH1_RUNNER_CONTEXT_SCHEMA,
  Path1RunnerContextAdapterError,
  buildOfficialInvocationPlan,
  buildStrictChildEnvironment,
  parseOptionalRunnerContext,
  runPath1RunnerContextAdapter,
  validateRunnerContextRequest,
} from "./path1-oss-runner-context-adapter.mjs"

const repoRoot = path.resolve(".")
const sourceHead = "a".repeat(40)
const cloudRevision = "b".repeat(40)
const runtimeDigest = `sha256:${"c".repeat(64)}`
let receiptNumber = 0

function context(overrides = {}) {
  const runId = "path1-adapter-test-001"
  const sessionId = "home-session-test-001"
  const roomId = "room-test-001"
  return {
    schema: PATH1_RUNNER_CONTEXT_SCHEMA,
    runId,
    allocationId: "allocation-test-001",
    machineId: "machine-test-001",
    homeKernelId: "home-kernel-test-001",
    homeRelayRealmId: "relay-realm-test-001",
    repoRoot,
    sourceHead,
    sessionId,
    roomId,
    kernelEndpoint: "ws://127.0.0.1:43119/kernel",
    relayEndpoint: "ws://127.0.0.1:43120/relay",
    webEndpoint: "http://127.0.0.1:4321",
    journal: { ref: "path1-journal-test-001", sequence: 4 },
    runtimeBindings: {
      cloudRevision,
      ossRevision: sourceHead,
      runtimeDigest,
      localDaemonProtocol: 333,
      relayPeerProtocol: 55,
    },
    surfaceManifest: {
      sameRoom: true,
      acceptsRunnerContext: true,
      normalKernelAuthority: true,
      relayTransportOnly: true,
    },
    clientModes: { web: true, localTui: true, remoteTui: true },
    clientBindings: Object.fromEntries(PATH1_RUNNER_CONTEXT_CLIENT_MODES.map((mode) => [mode, {
      runId,
      sessionId,
      roomId,
      authority: "home-kernel",
      attached: true,
    }])),
    authority: { kernel: "normal-kernel", relay: "transport-only", room: "kernel-owned" },
    officialSurfaces: {
      providerParity: path.join(repoRoot, "apps/cli/scripts/live-external-provider-live-parity-drill.mjs"),
      nativeTuiMatrix: path.join(repoRoot, "apps/cli/scripts/live-native-provider-tui-matrix-drill.mjs"),
      browserComputer: path.join(repoRoot, "apps/cli/scripts/live-room-environment-pointer-click-drill.mjs"),
      takeoverReconnect: path.join(repoRoot, "apps/cli/scripts/live-room-takeover-reconnect-fault-drill.mjs"),
    },
    ...overrides,
  }
}

function publicationReceipt() {
  return {
    schema: "chariox.path1.oss-publication-receipt.v1",
    status: "published",
    source: {
      cloudRevision,
      ossRevision: sourceHead,
      treeClean: true,
    },
    protocols: { localDaemon: 333, relayPeer: 55 },
    image: {
      digest: runtimeDigest,
      signatureVerification: "verified",
      attestationVerification: "verified",
      manifests: [{ platform: "linux/amd64", digest: runtimeDigest }],
    },
  }
}

function request(action, provider = null, overrides = {}) {
  return {
    schema: "chariox.path1.live-functional-driver-request.v1",
    action,
    provider,
    runId: context().runId,
    sourceHead,
    contract: {
      ossRevision: sourceHead,
      localDaemonProtocolVersion: 333,
      relayPeerProtocolVersion: 55,
    },
    publicationReceipt: publicationReceipt(),
    runnerContext: context(overrides),
  }
}

function clientFactoryFor(events, overrides = {}) {
  return {
    async attach(value) {
      events.push({ type: "attach", ...value })
      return {
        binding: {
          runId: value.runId,
          sessionId: value.sessionId,
          roomId: value.roomId,
          authority: "home-kernel",
        },
      }
    },
    async verifyRoom(value) {
      events.push({ type: "verify-room", ...value })
      return {
        runId: value.runId,
        sessionId: value.sessionId,
        roomId: value.roomId,
        kernelAuthoritative: true,
        relayTransportOnly: true,
      }
    },
    async close() { events.push({ type: "close" }) },
    ...overrides,
  }
}

function liveEvidence(current, extras = {}) {
  return {
    schema: "chariox.path1.runner-context-result.v1",
    source: "deployed-oss-live-drill",
    liveObserved: true,
    dryRun: false,
    sourceTestOnly: false,
    runId: current.runId,
    sourceHead,
    sessionId: current.sessionId,
    roomId: current.roomId,
    official: true,
    kernelAuthoritative: true,
    relayTransportOnly: true,
    receiptId: `receipt-${String(++receiptNumber).padStart(3, "0")}`,
    ...extras,
  }
}

function journalStore() {
  const state = { events: [] }
  return {
    state,
    async append(event) { state.events.push(event) },
    async load() {
      const results = {}
      for (const event of state.events) if (event.type === "official-cell") results[event.invocationId] = event.result
      return { results }
    },
  }
}

test("plan is complete, browser-first, standalone-free, and does not attach clients", async () => {
  const current = context()
  const result = await runPath1RunnerContextAdapter({ input: request("capability-drills"), mode: "plan" })
  assert.equal(result.schema, "chariox.path1.runner-context-result.v1")
  assert.deepEqual(result.invocationOrder, [
    "capability:browser",
    "capability:computer-fallback",
    "capability:takeover-reconnect",
  ])
  assert.equal(result.browserFirst, true)
  assert.equal(result.sameRoomRequired, true)
  assert.equal(result.sessionId, current.sessionId)
  assert.equal(result.roomId, current.roomId)
})

test("apply performs the exact handshake and binds Web/local-TUI/remote-TUI to one Room", async () => {
  const events = []
  const journal = journalStore()
  const result = await runPath1RunnerContextAdapter({
    input: request("context-transfer"),
    mode: "apply",
    confirmed: true,
    clientFactory: clientFactoryFor(events),
    journalWriter: journal,
  })
  assert.equal(result.action, "context-transfer")
  assert.equal(result.kernelAuthoritative, true)
  assert.equal(result.relayTransportOnly, true)
  assert.deepEqual(events.filter((event) => event.type === "attach").map((event) => event.mode), ["web", "localTui", "remoteTui"])
  assert.ok(events.filter((event) => event.type === "attach").every((event) => event.sessionId === "home-session-test-001" && event.roomId === "room-test-001"))
  assert.equal(events.filter((event) => event.type === "close").length, 1)
  assert.deepEqual(journal.state.events.map((event) => event.type), ["runner-context-attached", "runner-context-verified"])
})

test("provider cells run all providers with Browser before Computer fallback", async () => {
  const events = []
  const invocationIds = []
  const current = context()
  const processRunner = {
    async run(invocation) {
      invocationIds.push(invocation.id)
      const fallback = invocation.computerFallback
      return liveEvidence(current, {
        provider: invocation.provider,
        actorId: "actor-provider-001",
        threadId: "thread-provider-001",
        surfaces: [
          "apps/cli/scripts/live-external-provider-live-parity-drill.mjs",
          "apps/cli/scripts/live-native-provider-tui-matrix-drill.mjs",
        ],
        ...(fallback ? {
          computerFallback: { structured: true, completed: true, receiptId: "computer-codex-001" },
        } : {
          browser: { structured: true, completed: true, receiptId: "browser-codex-001" },
        }),
        completion: { state: "completed", exactlyOnce: true, count: 1 },
        history: { durable: true, receiptId: "history-codex-001" },
      })
    },
  }
  for (const provider of PATH1_RUNNER_CONTEXT_PROVIDERS) {
    const result = await runPath1RunnerContextAdapter({
      input: request("provider-cell", provider),
      mode: "apply",
      confirmed: true,
      clientFactory: clientFactoryFor(events),
      processRunner,
    })
    assert.equal(result.provider, provider)
    assert.equal(result.browser.structured, true)
    assert.equal(result.computerFallback.completed, true)
  }
  assert.deepEqual(invocationIds.map((id) => id.split(":").slice(-1)[0]), [
    "browser", "computer-fallback", "browser", "computer-fallback", "browser", "computer-fallback",
  ])
})

test("capability result preserves reconnect, persistence, failure, screenshot/OCR, input, permission, takeover, and history evidence", async () => {
  const current = context()
  const processRunner = {
    async run(invocation) {
      const capability = {
        browser: { completed: true, screenshotOcr: { screenshot: true, ocr: true } },
        computer: { completed: true, pointer: true, keyboard: true, clipboard: true },
        reconnect: { reconnected: true, persistence: true, history: true, failureRecovered: true },
      }
      return liveEvidence(current, { capabilities: capability, receiptId: `receipt-${invocation.id.replaceAll(":", "-")}` })
    },
  }
  const result = await runPath1RunnerContextAdapter({
    input: request("capability-drills"),
    mode: "apply",
    confirmed: true,
    clientFactory: clientFactoryFor([]),
    processRunner,
  })
  assert.equal(result.capabilities?.browser?.screenshotOcr?.ocr, true)
  assert.equal(result.capabilities?.computer?.clipboard, true)
  assert.equal(result.capabilities?.reconnect?.failureRecovered, true)
})

test("interruption preserves the runner journal and resumes completed official cells without replaying them", async () => {
  const current = context()
  const journal = journalStore()
  const firstCalls = []
  await assert.rejects(
    () => runPath1RunnerContextAdapter({
      input: request("capability-drills"),
      mode: "apply",
      confirmed: true,
      clientFactory: clientFactoryFor([]),
      journalWriter: journal,
      processRunner: {
        async run(invocation) {
          firstCalls.push(invocation.id)
          if (invocation.id === "capability:computer-fallback") {
            const error = new Error("injected interruption")
            error.code = "interrupted"
            throw error
          }
          return liveEvidence(current, { capabilities: { browser: true }, receiptId: "browser-complete-001" })
        },
      },
    }),
    /injected interruption/u,
  )
  assert.deepEqual(firstCalls, ["capability:browser", "capability:computer-fallback"])
  assert.ok(journal.state.events.some((event) => event.type === "runner-context-failure"))
  const resumedCalls = []
  const result = await runPath1RunnerContextAdapter({
    input: request("capability-drills"),
    mode: "apply",
    confirmed: true,
    clientFactory: clientFactoryFor([]),
    journalWriter: journal,
    processRunner: {
      async run(invocation) {
        resumedCalls.push(invocation.id)
        return liveEvidence(current, { capabilities: { browser: true, computer: true, reconnect: true }, receiptId: `receipt-${invocation.id.replaceAll(":", "-")}` })
      },
    },
  })
  assert.deepEqual(resumedCalls, ["capability:computer-fallback", "capability:takeover-reconnect"])
  assert.equal(result.liveObserved, true)
})

test("stale or mismatched context, missing context, and secrets fail closed before a child can run", async () => {
  assert.throws(
    () => {
      const missing = request("context-transfer")
      delete missing.runnerContext
      return validateRunnerContextRequest(missing)
    },
    /runnerContext: must be an object/u,
  )
  await assert.rejects(
    () => runPath1RunnerContextAdapter({ input: request("context-transfer", null, { roomId: "different-room-001" }), mode: "plan" }),
    /does not bind to the supplied Room\/session/u,
  )
  await assert.rejects(
    () => runPath1RunnerContextAdapter({ input: { ...request("context-transfer"), runnerContext: { ...context(), providerToken: "never" } }, mode: "plan" }),
    /credential material/u,
  )
})

test("strict child environment carries only safe linkage and keeps manual mode context-free", () => {
  const current = validateRunnerContextRequest(request("provider-cell", "codex"))
  const environment = buildStrictChildEnvironment(current, {
    PATH: "/usr/bin",
    HOME: "/safe/home",
    CHARIOX_RELAY_TOKEN: "must-not-propagate",
    PROVIDER_API_KEY: "must-not-propagate",
  })
  assert.equal(environment.PATH, "/usr/bin")
  assert.equal(environment.HOME, "/safe/home")
  assert.equal(environment.CHARIOX_RELAY_TOKEN, undefined)
  assert.equal(environment.PROVIDER_API_KEY, undefined)
  assert.equal(environment.CHARIOX_PATH1_SOURCE_HEAD, sourceHead)
  assert.equal(parseOptionalRunnerContext(undefined), null)
  assert.equal(parseOptionalRunnerContext(""), null)
  assert.ok(buildOfficialInvocationPlan(current).every((invocation) => invocation.args.includes("--path1-runner-context")))
})
