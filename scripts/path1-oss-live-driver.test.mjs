import assert from "node:assert/strict"
import { mkdtempSync, rmSync } from "node:fs"
import { tmpdir } from "node:os"
import path from "node:path"
import test from "node:test"

import {
  PATH1_OSS_LIVE_DRIVER_SCHEMA,
  PATH1_OSS_LIVE_SOURCE,
  PATH1_PROTOCOLS,
  PATH1_REQUIRED_PROVIDERS,
  PATH1_REQUIRED_SURFACES,
  createMemoryJournalStore,
  runPath1OssLiveAcceptance,
  runPath1OssLiveAction,
} from "./path1-oss-live-driver.mjs"

const repoRoot = path.resolve(".")
const sourceHead = "a".repeat(40)
const cloudHead = "b".repeat(40)
const digest = `sha256:${"c".repeat(64)}`
let fixtureNumber = 0

function fixture(overrides = {}) {
  const scratch = mkdtempSync(path.join(tmpdir(), "chariox-path1-oss-live-driver-test-"))
  const runId = `path1-test-${String(++fixtureNumber).padStart(3, "0")}`
  const context = {
    runId,
    allocationId: "allocation-test-001",
    machineId: "machine-test-001",
    homeKernelId: "home-kernel-test-001",
    homeRelayRealmId: "relay-realm-test-001",
    repoRoot,
    journalPath: path.join(scratch, "journal.json"),
    evidenceRoot: path.join(scratch, "evidence"),
    surfaceManifest: {
      sameRoom: true,
      acceptsRunnerContext: true,
      normalKernelAuthority: true,
      relayTransportOnly: true,
    },
  }
  const input = {
    schema: PATH1_OSS_LIVE_DRIVER_SCHEMA,
    runId,
    allocationId: context.allocationId,
    machineId: context.machineId,
    homeKernelId: context.homeKernelId,
    homeRelayRealmId: context.homeRelayRealmId,
    sourceHead,
    contract: {
      ossRevision: sourceHead,
      localDaemonProtocolVersion: PATH1_PROTOCOLS.localDaemon,
      relayPeerProtocolVersion: PATH1_PROTOCOLS.relayPeer,
    },
    publicationReceipt: {
      schema: "chariox.path1.oss-publication-receipt.v1",
      status: "published",
      source: {
        cloudRevision: cloudHead,
        ossRevision: sourceHead,
        treeClean: true,
        revisionLabel: { name: "io.chariox.runtime-source-revision", value: sourceHead },
      },
      protocols: { localDaemon: 333, relayPeer: 55 },
      image: {
        reference: `ghcr.io/charioxai/path1-runtime@${digest}`,
        digest,
        signatureVerification: "verified",
        attestationVerification: "verified",
        manifests: [{ platform: "linux/amd64", digest }],
      },
      publication: { status: "succeeded", quarantine: "none" },
    },
    contextPlan: {
      contextId: "context-test-001",
      planDigest: digest,
      kernelContext: "empty",
    },
    runnerContext: context,
    ...overrides,
  }
  return { input, scratch }
}

function commonEvidence(action, payload) {
  return {
    source: PATH1_OSS_LIVE_SOURCE,
    liveObserved: true,
    dryRun: false,
    sourceTestOnly: false,
    action,
    status: action === "project-environment-ready" ? "ready" : "completed",
    runId: payload.runId,
    sourceHead: payload.sourceHead,
    allocationId: payload.allocationId,
    machineId: payload.machineId,
    sessionId: payload.sessionId ?? "session-test-001",
    roomId: payload.roomId ?? "room-test-001",
    receiptId: `receipt-${action}-${payload.provider ?? "none"}`,
  }
}

function providerEvidence(provider, payload) {
  return {
    ...commonEvidence("provider-cell", payload),
    provider,
    providerId: provider,
    official: true,
    officialHarness: provider,
    actorId: `actor-${provider}-001`,
    threadId: `thread-${provider}-001`,
    surfaces: [...PATH1_REQUIRED_SURFACES],
    browser: { structured: true, completed: true, receiptId: `browser-${provider}-001` },
    computerFallback: { structured: true, completed: true, receiptId: `computer-${provider}-001` },
    completion: { state: "completed", exactlyOnce: true, count: 1 },
    history: { durable: true, receiptId: `history-${provider}-001` },
  }
}

function capabilityEvidence(payload) {
  const attachment = (surface) => ({
    applicable: true,
    attached: true,
    runId: payload.runId,
    sessionId: payload.sessionId,
    roomId: payload.roomId,
    receiptId: `attach-${surface}-001`,
  })
  const input = (kind) => ({ ok: true, runId: payload.runId, sessionId: payload.sessionId, roomId: payload.roomId, receiptId: `${kind}-001` })
  return {
    ...commonEvidence("capability-drills", payload),
    surfaces: [...PATH1_REQUIRED_SURFACES],
    topology: {
      attachments: { web: attachment("web"), localTui: attachment("localTui"), remoteTui: attachment("remoteTui") },
      readiness: {
        browserControllerReady: true,
        streamReady: true,
        browserControllerReceiptId: "browser-ready-001",
        streamReceiptId: "stream-ready-001",
      },
    },
    functional: {
      input: { pointer: input("pointer"), keyboard: input("keyboard"), clipboard: input("clipboard") },
      screenshotOcr: { screenshot: input("screenshot"), ocr: input("ocr") },
      takeover: {
        requested: true,
        granted: true,
        agentBlocked: true,
        idempotentReplay: true,
        sameRoom: true,
        idempotencyKey: "takeover-idempotency-001",
        actionId: "takeover-action-001",
        replayActionId: "takeover-action-001",
        receiptId: "takeover-001",
      },
      permission: { requested: true, interacted: true, resolved: true, outcome: "approved", receiptId: "permission-001" },
      reconnectReload: { reconnected: true, reloaded: true, sameRoom: true, sameSession: true, providerThreadContinuity: true, receiptId: "reconnect-001" },
    },
    history: { durable: true, sameRoom: true, providerThreadsPersisted: true, entryCount: 3, receiptId: "history-all-001" },
    saveRestartReconnect: {
      saved: true,
      restarted: true,
      reconnected: true,
      sameRoom: true,
      sameSession: true,
      providerThreadContinuity: true,
      receiptId: "restart-001",
      providerThreads: Object.fromEntries(PATH1_REQUIRED_PROVIDERS.map((provider) => [provider, { before: `thread-${provider}-001`, after: `thread-${provider}-001`, same: true }])),
    },
    idle: { intervalMs: 1_000, sameProviderThreads: true, resumedRequest: { completed: true, exactlyOnce: true, sameRoom: true, receiptId: "idle-001" } },
    failureInjection: { injected: true, errorObserved: true, recovered: true, bounded: true, sameRoom: true, noDuplicateCompletion: true, kind: "bounded-reconnect", durationMs: 20, limitMs: 1000, receiptId: "failure-001" },
    resources: {
      withinLimits: true,
      receiptId: "resources-001",
      limits: { vmBootMs: 1, browserReadyMs: 1, streamReadyMs: 1, screenshotOcrMs: 1, resumedRequestMs: 1, rssMb: 1, diskDeltaMb: 1 },
      samples: { vmBootMs: 0, browserReadyMs: 0, streamReadyMs: 0, screenshotOcrMs: 0, resumedRequestMs: 0, rssMb: 0, diskDeltaMb: 0 },
    },
  }
}

function makeRunner({ failAt = null, mutate = null } = {}) {
  const calls = []
  const cleanupCalls = []
  const runner = {
    kind: "injected",
    calls,
    cleanupCalls,
    async invoke({ action, provider, payload }) {
      calls.push({ action, provider, payload })
      if (failAt?.(action, provider)) {
        const error = new Error("injected interruption")
        error.code = "interrupted"
        throw error
      }
      let result
      if (action === "context-transfer") {
        result = {
          ...commonEvidence(action, payload),
          status: "completed",
          environmentId: "environment-test-001",
          contextId: payload.contextPlan.contextId,
          planDigest: payload.contextPlan.planDigest,
          kernelAuthoritative: true,
          relayTransportOnly: true,
          userActorId: "user-actor-test-001",
        }
      } else if (action === "project-environment-ready") {
        result = { ...commonEvidence(action, payload), automatic: true, environmentId: payload.allocationId }
      } else if (action === "provider-cell") {
        result = providerEvidence(provider, payload)
      } else {
        result = capabilityEvidence(payload)
      }
      return mutate ? mutate(result, { action, provider, payload }) : result
    },
    async cleanupOwned(value) { cleanupCalls.push(value) },
  }
  return runner
}

const sourceInspector = async () => ({ head: sourceHead, dirty: false })

function cleanup(scratch) {
  rmSync(scratch, { recursive: true, force: true })
}

test("plan mode exposes the complete ordered live sequence without invoking a seam", async () => {
  const { input, scratch } = fixture()
  const runner = makeRunner()
  try {
    const result = await runPath1OssLiveAcceptance({ input, mode: "plan", runner, sourceInspector })
    assert.equal(result.status, "plan")
    assert.deepEqual(result.plan.steps.map((step) => step.id), [
      "validate-publication-receipt",
      "verify-exact-source-head",
      "journal-run-intent",
      "bind-normal-kernel-relay-room",
      "transfer-context-through-home-kernel-and-relay",
      "wait-for-automatic-project-environment-ready",
      "run-codex-official-live-cell",
      "run-opencode-official-live-cell",
      "run-claude-official-live-cell",
      "run-browser-first-computer-fallback-capability-cells",
      "sample-live-resources",
      "journal-sanitized-live-evidence",
      "emit-sanitized-per-cell-result",
    ])
    assert.equal(result.plan.liveObserved, false)
    assert.deepEqual(runner.calls, [])
  } finally {
    cleanup(scratch)
  }
})

test("apply runs one ordered Room authority sequence for every provider and capability cell", async () => {
  const { input, scratch } = fixture()
  const runner = makeRunner()
  try {
    const result = await runPath1OssLiveAcceptance({ input, mode: "apply", confirmed: true, runner, sourceInspector })
    assert.equal(result.status, "passed")
    assert.deepEqual(runner.calls.map(({ action, provider }) => provider ? `${action}:${provider}` : action), [
      "context-transfer",
      "project-environment-ready",
      "provider-cell:codex",
      "provider-cell:opencode",
      "provider-cell:claude",
      "capability-drills",
    ])
    assert.equal(result.receipt.liveObserved, true)
    assert.equal(result.receipt.source, PATH1_OSS_LIVE_SOURCE)
    assert.equal(Object.keys(result.receipt.providers).length, 3)
    assert.equal(runner.cleanupCalls.length, 1)
    assert.equal(runner.cleanupCalls[0].scope, "drill-owned")
    assert.ok(runner.cleanupCalls[0].processes.every((entry) => entry.id.startsWith("owned-")))
  } finally {
    cleanup(scratch)
  }
})

test("apply rejects non-live or source-test-only evidence instead of manufacturing success", async () => {
  const { input, scratch } = fixture()
  const runner = makeRunner({ mutate: (result, { action }) => action === "provider-cell" ? { ...result, liveObserved: false, sourceTestOnly: true } : result })
  try {
    const result = await runPath1OssLiveAcceptance({ input, mode: "apply", confirmed: true, runner, sourceInspector })
    assert.equal(result.status, "failed")
    assert.equal(result.failure.field, "live.provider-cell")
    assert.equal(result.journal.status, "failed")
  } finally {
    cleanup(scratch)
  }
})

test("the real adapter fails closed when the supplied context has no existing Room", async () => {
  const { input, scratch } = fixture()
  try {
    const result = await runPath1OssLiveAcceptance({ input, mode: "apply", confirmed: true, sourceInspector })
    assert.equal(result.status, "failed")
    assert.equal(result.failure.code, "missing-product-seam")
    assert.match(result.failure.field, /^runnerContext\./u)
  } finally {
    cleanup(scratch)
  }
})

test("an interrupted provider cell resumes idempotently without repeating completed cells", async () => {
  const { input, scratch } = fixture()
  const store = createMemoryJournalStore()
  const first = makeRunner({ failAt: (action, provider) => action === "provider-cell" && provider === "opencode" })
  const second = makeRunner()
  try {
    const interrupted = await runPath1OssLiveAcceptance({ input, mode: "apply", confirmed: true, runner: first, journalStore: store, sourceInspector })
    assert.equal(interrupted.status, "interrupted")
    assert.equal(interrupted.journal.stages["provider:codex"], true)
    assert.equal(interrupted.journal.stages["provider:opencode"], undefined)
    const resumed = await runPath1OssLiveAcceptance({ input, mode: "apply", confirmed: true, runner: second, journalStore: store, sourceInspector })
    assert.equal(resumed.status, "passed")
    assert.deepEqual(second.calls.map(({ action, provider }) => provider ? `${action}:${provider}` : action), [
      "provider-cell:opencode",
      "provider-cell:claude",
      "capability-drills",
    ])
    assert.equal(resumed.resumed, true)
    assert.equal(resumed.journal.status, "complete")
  } finally {
    cleanup(scratch)
  }
})

test("failure preserves owned journal evidence and never invokes cleanup for an incomplete run", async () => {
  const { input, scratch } = fixture()
  const runner = makeRunner({ failAt: (action, provider) => action === "provider-cell" && provider === "claude" })
  try {
    const result = await runPath1OssLiveAcceptance({ input, mode: "apply", confirmed: true, runner, sourceInspector })
    assert.equal(result.status, "interrupted")
    assert.equal(runner.cleanupCalls.length, 0)
    assert.ok(result.journal.owned.processes.length >= 5)
    assert.ok(result.journal.owned.artifacts.length >= 1)
    assert.equal(result.journal.owned.scope, "drill-owned")
    assert.equal(result.journal.failure.code, "interrupted")
  } finally {
    cleanup(scratch)
  }
})

test("redaction rejects secret-bearing context and never returns secret-bearing live evidence", async () => {
  const secretFixture = fixture()
  secretFixture.input.runnerContext.apiToken = "do-not-print"
  try {
    await assert.rejects(
      () => runPath1OssLiveAcceptance({ input: secretFixture.input, mode: "plan", sourceInspector }),
      /inline secret material/u,
    )
  } finally {
    cleanup(secretFixture.scratch)
  }
  const { input, scratch } = fixture()
  const runner = makeRunner({ mutate: (result, { action }) => action === "provider-cell" ? { ...result, redactedToken: "Bearer secret-value" } : result })
  try {
    const result = await runPath1OssLiveAcceptance({ input, mode: "apply", confirmed: true, runner, sourceInspector })
    assert.equal(result.status, "failed")
    assert.doesNotMatch(JSON.stringify(result), /secret-value/u)
  } finally {
    cleanup(scratch)
  }
})

test("single action protocol preserves the Cloud-consumable provider evidence shape", async () => {
  const { input, scratch } = fixture({
    action: "provider-cell",
    provider: "codex",
    sessionId: "session-test-001",
    roomId: "room-test-001",
  })
  const runner = makeRunner()
  try {
    const result = await runPath1OssLiveAction({ input, mode: "apply", confirmed: true, runner, sourceInspector })
    assert.equal(result.action, "provider-cell")
    assert.equal(result.provider, "codex")
    assert.equal(result.liveObserved, true)
    assert.equal(result.browser.structured, true)
    assert.equal(result.computerFallback.completed, true)
  } finally {
    cleanup(scratch)
  }
})

test("single action apply returns a sanitized missing-seam failure", async () => {
  const { input, scratch } = fixture({
    action: "provider-cell",
    provider: "codex",
    sessionId: "session-test-001",
    roomId: "room-test-001",
  })
  try {
    const result = await runPath1OssLiveAction({ input, mode: "apply", confirmed: true, sourceInspector })
    assert.equal(result.status, "failed")
    assert.equal(result.failure.code, "missing-product-seam")
    assert.equal(result.journal.status, "failed")
    assert.doesNotMatch(JSON.stringify(result), /runner-context-capable-same-Room-adapter/u)
  } finally {
    cleanup(scratch)
  }
})

test("publication receipt source head is authoritative and mismatches fail before a runner call", async () => {
  const { input, scratch } = fixture({ sourceHead: "d".repeat(40) })
  const runner = makeRunner()
  try {
    await assert.rejects(
      () => runPath1OssLiveAcceptance({ input, mode: "plan", runner, sourceInspector }),
      /conflicting source heads|sourceHead|contract\.ossRevision/u,
    )
    assert.deepEqual(runner.calls, [])
  } finally {
    cleanup(scratch)
  }
})
