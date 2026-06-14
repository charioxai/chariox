import assert from "node:assert/strict"
import { mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  finalizeDrillArtifacts,
  prepareDrillArtifacts,
} from "./drill-artifacts.mjs"
import {
  classifyDrillFailureManifest,
  findDrillFailureManifestPaths,
  formatDrillFailureManifestAggregateSummary,
  formatDrillFailureManifestSummary,
  readDrillFailureManifest,
  summarizeDrillFailureManifest,
  summarizeDrillFailureManifests,
  validateDrillFailureManifest,
} from "./drill-failure-manifest.mjs"

test("reads and summarizes a preserved drill failure directory", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-failure-summary-"))
  await prepareDrillArtifacts(root)

  await finalizeDrillArtifacts({
    rootDir: root,
    passed: false,
    failure: new Error("relay target was stale"),
    metadata: {
      drill: "hosted-cloud-relay",
      provider: "opencode-zen",
      runtimeSignals: "relay-target-freshness,lease-health",
      token: "should-not-print",
      nested: { ignored: true },
    },
  })

  const manifest = await readDrillFailureManifest(root)
  const summary = summarizeDrillFailureManifest(manifest, { source: root })
  const text = formatDrillFailureManifestSummary(manifest, { source: root })

  assert.equal(manifest.metadata.token, "<redacted>")
  assert.equal(summary.schema, "arroba.drill.failure.v1")
  assert.equal(summary.metadata.drill, "hosted-cloud-relay")
  assert.equal(summary.metadata.provider, "opencode-zen")
  assert.equal(summary.metadata.runtimeSignals, "relay-target-freshness,lease-health")
  assert.equal(summary.metadata.token, "<redacted>")
  assert.equal(summary.metadata.nested, undefined)
  assert.equal(summary.error.name, "Error")
  assert.equal(summary.error.message, "relay target was stale")
  assert.equal(summary.classification.kind, "relay-runtime")
  assert.equal(summary.classification.owner, "runtime-network")
  assert.match(text, /drill failure: hosted-cloud-relay/)
  assert.match(text, /metadata: provider=opencode-zen runtimeSignals=relay-target-freshness,lease-health token=<redacted>/)
  assert.match(text, /error=Error: relay target was stale/)
  assert.match(text, /owner=runtime-network classification=relay-runtime/)
  assert.doesNotMatch(text, /should-not-print/)

  await rm(root, { recursive: true, force: true })
})

test("reads a manifest file directly", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-failure-file-"))
  const file = path.join(root, "failure.json")
  await writeFile(file, `${JSON.stringify(validManifest({ rootDir: root }))}\n`, "utf8")

  const manifest = await readDrillFailureManifest(file)

  assert.equal(manifest.rootDir, root)
  await rm(root, { recursive: true, force: true })
})

test("redacts token-shaped values from summarized manifest errors", () => {
  const summary = summarizeDrillFailureManifest(validManifest({
    error: {
      name: "Error",
      message: "Token refresh failed: 401 with Bearer abcdefghijklmnopqrstuvwxyz",
      stack: null,
    },
  }))

  assert.equal(summary.error.message, "Token refresh failed: 401 with <redacted>")
})

test("discovers preserved failure manifests below artifact roots", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-failure-find-"))
  const first = path.join(root, "target", "run-one")
  const second = path.join(root, ".artifacts", "run-two")
  const ignored = path.join(root, "node_modules", "run-three")
  await prepareDrillArtifacts(first)
  await prepareDrillArtifacts(second)
  await prepareDrillArtifacts(ignored)
  await finalizeDrillArtifacts({ rootDir: first, passed: false, failure: new Error("first"), metadata: { drill: "first" } })
  await finalizeDrillArtifacts({ rootDir: second, passed: false, failure: new Error("second"), metadata: { drill: "second" } })
  await finalizeDrillArtifacts({ rootDir: ignored, passed: false, failure: new Error("ignored"), metadata: { drill: "ignored" } })

  const manifests = await findDrillFailureManifestPaths(root)

  assert.deepEqual(manifests, [
    path.join(root, ".artifacts", "run-two", "arroba-drill-failure.json"),
    path.join(root, "target", "run-one", "arroba-drill-failure.json"),
  ])
  await rm(root, { recursive: true, force: true })
})

test("rejects malformed failure manifests", () => {
  assert.throws(() => validateDrillFailureManifest({}), /unsupported schema/)
  assert.throws(() => validateDrillFailureManifest({
    ...validManifest(),
    rootDir: "",
  }), /missing rootDir/)
  assert.throws(() => validateDrillFailureManifest({
    ...validManifest(),
    failedAt: "2026-06-13",
  }), /failedAt must be an ISO timestamp/)
  assert.throws(() => validateDrillFailureManifest({
    ...validManifest(),
    metadata: [],
  }), /invalid metadata/)
  assert.throws(() => validateDrillFailureManifest({
    ...validManifest(),
    metadata: { provider: "Bearer abcdefghijklmnopqrstuvwxyz" },
  }), /secret-looking metadata value/)
  assert.throws(() => validateDrillFailureManifest({
    ...validManifest(),
    metadata: { nested: { provider: "sk-ant-abcdefghijklmnopqrstuvwxyz123456" } },
  }), /secret-looking metadata value/)
  assert.throws(() => validateDrillFailureManifest({
    ...validManifest(),
    metadata: { runtimeSignals: "workspace-live-synch-state" },
  }), /metadata\.runtimeSignals\[0\] has unknown runtime signal "workspace-live-synch-state"/)
  assert.throws(() => validateDrillFailureManifest({
    ...validManifest(),
    metadata: { runtimeSignalOwners: "kernel-authority" },
  }), /runtimeSignalOwners requires runtimeSignals/)
  assert.throws(() => validateDrillFailureManifest({
    ...validManifest(),
    metadata: {
      runtimeSignals: "session-authority,provider-run-lifecycle",
      runtimeSignalOwners: "kernel-authority",
    },
  }), /runtimeSignalOwners must match runtimeSignals/)
  assert.throws(() => validateDrillFailureManifest({
    ...validManifest(),
    error: { name: "Error", message: "failed", stack: 1 },
  }), /invalid stack/)
})

test("classifies common drill failure owners and next actions", () => {
  assert.deepEqual(classifyDrillFailureManifest(validManifest({
    error: { name: "Error", message: "Token refresh failed: 401", stack: null },
  })), {
    kind: "provider-auth",
    owner: "provider-account",
    nextAction: "refresh provider login for the profile used by this drill, then rerun the drill",
  })

  assert.deepEqual(classifyDrillFailureManifest(validManifest({
    error: { name: "Error", message: "Docker is required for the slice lifecycle drill. Start Docker/Colima and retry.", stack: null },
  })), {
    kind: "docker-runtime",
    owner: "local-machine",
    nextAction: "start Docker or Colima, confirm `docker info` succeeds, then rerun the drill",
  })

  assert.deepEqual(classifyDrillFailureManifest(validManifest({
    metadata: { drill: "remote-restart", relayUrl: "ws://127.0.0.1:43385" },
    error: { name: "Error", message: "spawn pnpm ENOENT", stack: null },
  })), {
    kind: "test-harness",
    owner: "validation-harness",
    nextAction: "install or build the missing local drill prerequisite, then rerun the drill",
  })

  assert.deepEqual(classifyDrillFailureManifest(validManifest({
    error: { name: "Error", message: "deployment did not become ready: Scalingo 503 service unavailable", stack: null },
  })), {
    kind: "cloud-runtime",
    owner: "cloud-deployment",
    nextAction: "inspect Cloud deployment/control-plane status and preserved logs, then rerun the drill",
  })

  assert.deepEqual(classifyDrillFailureManifest(validManifest({
    error: { name: "Error", message: "timed out waiting for provider run run-1 to become ready\nlast_observation={\"state\":\"Starting\"}", stack: null },
  })), {
    kind: "runtime-timeout",
    owner: "runtime-state",
    nextAction: "inspect preserved runtime state, provider run lifecycle, and drill timeout diagnostics, then rerun the drill",
  })

  assert.deepEqual(classifyDrillFailureManifest(validManifest({
    error: { name: "Error", message: "workspace live sync result skipped_conflict for src/app.ts", stack: null },
  })), {
    kind: "workspace-live-sync-conflict",
    owner: "runtime-state",
    nextAction: "inspect workspace live sync status, conflicts, and preserved file snapshots; reconcile the conflict, then rerun the drill",
  })

  assert.deepEqual(classifyDrillFailureManifest(validManifest({
    error: { name: "Error", message: "remote worker execution failed: leased agent agent-1 failed to launch provider run", stack: null },
  })), {
    kind: "worker-execution",
    owner: "worker-kernel",
    nextAction: "inspect worker kernel logs, leased-agent launch state, and preserved worker artifacts, then rerun the drill",
  })

  assert.deepEqual(classifyDrillFailureManifest(validManifest({
    error: { name: "Error", message: "web terminal projection failed: terminal event was not rendered", stack: null },
  })), {
    kind: "ui-client-projection",
    owner: "ui-client",
    nextAction: "inspect web/TUI terminal projection logs, transcript rendering state, and preserved screenshots or terminal captures, then rerun the drill",
  })

  assert.deepEqual(classifyDrillFailureManifest(validManifest({
    error: { name: "Error", message: "projection invariant failed: session prompt read-model stale", stack: null },
  })), {
    kind: "projection-staleness",
    owner: "kernel-authority",
    nextAction: "inspect kernel projection health, read-model freshness, and reconciliation events before rerunning the drill",
  })
})

test("aggregates preserved drill failure summaries", () => {
  const relay = validManifest({
    rootDir: "/tmp/relay",
    metadata: { drill: "relay-drill", runtimeSignals: "relay-target-freshness,lease-health" },
    error: { name: "Error", message: "relay target stale", stack: null },
  })
  const provider = validManifest({
    rootDir: "/tmp/provider",
    metadata: { drill: "provider-drill", runtimeSignals: "provider-run-lifecycle,lease-health" },
    error: { name: "Error", message: "Token refresh failed: 401", stack: null },
  })

  const aggregate = summarizeDrillFailureManifests([relay, provider], {
    sources: ["/tmp/relay/arroba-drill-failure.json", "/tmp/provider/arroba-drill-failure.json"],
  })

  assert.equal(aggregate.schema, "arroba.drill.failure.aggregate.v1")
  assert.equal(aggregate.total, 2)
  assert.deepEqual(aggregate.owners, { "provider-account": 1, "runtime-network": 1 })
  assert.deepEqual(aggregate.classifications, { "provider-auth": 1, "relay-runtime": 1 })
  assert.deepEqual(aggregate.runtimeSignals, {
    "lease-health": 2,
    "provider-run-lifecycle": 1,
    "relay-target-freshness": 1,
  })
  assert.deepEqual(aggregate.runtimeSignalOwners, {
    "kernel-authority": 2,
    "provider-runtime": 1,
    "runtime-network": 1,
  })
  assert.deepEqual(aggregate.nextActions.map((action) => ({
    owner: action.owner,
    classification: action.classification,
    count: action.count,
  })), [
    { owner: "provider-account", classification: "provider-auth", count: 1 },
    { owner: "runtime-network", classification: "relay-runtime", count: 1 },
  ])
  assert.deepEqual(aggregate.failures.map((failure) => ({
    drill: failure.drill,
    source: failure.source,
    owner: failure.owner,
    classification: failure.classification,
    runtimeSignals: failure.runtimeSignals,
  })), [
    {
      drill: "relay-drill",
      source: "/tmp/relay/arroba-drill-failure.json",
      owner: "runtime-network",
      classification: "relay-runtime",
      runtimeSignals: ["lease-health", "relay-target-freshness"],
    },
    {
      drill: "provider-drill",
      source: "/tmp/provider/arroba-drill-failure.json",
      owner: "provider-account",
      classification: "provider-auth",
      runtimeSignals: ["lease-health", "provider-run-lifecycle"],
    },
  ])

  const text = formatDrillFailureManifestAggregateSummary(aggregate)
  assert.match(text, /drill failure aggregate:/)
  assert.match(text, /owners: provider-account=1 runtime-network=1/)
  assert.match(text, /classifications: provider-auth=1 relay-runtime=1/)
  assert.match(text, /runtime_signals: lease-health=2 provider-run-lifecycle=1 relay-target-freshness=1/)
  assert.match(text, /runtime_signal_owners: kernel-authority=2 provider-runtime=1 runtime-network=1/)
  assert.match(text, /next actions:/)
  assert.match(text, /owner=provider-account classification=provider-auth count=1: refresh provider login/)
  assert.match(text, /- relay-drill owner=runtime-network classification=relay-runtime runtime_signals=lease-health,relay-target-freshness root=\/tmp\/relay source=\/tmp\/relay\/arroba-drill-failure.json/)
  assert.match(text, /next: inspect relay and kernel logs/)
})

test("rejects inconsistent failure aggregates", () => {
  const aggregate = summarizeDrillFailureManifests([
    validManifest({
      rootDir: "/tmp/provider",
      metadata: { drill: "provider-drill" },
      error: { name: "Error", message: "Token refresh failed: 401", stack: null },
    }),
  ])

  assert.throws(() => formatDrillFailureManifestAggregateSummary({
    ...aggregate,
    total: 2,
  }), /total does not match failures/)
  assert.throws(() => formatDrillFailureManifestAggregateSummary({
    ...aggregate,
    owners: { "runtime-network": 1 },
  }), /owners do not match failures/)
  assert.throws(() => formatDrillFailureManifestAggregateSummary({
    ...aggregate,
    nextActions: [],
  }), /nextActions do not match failures/)
  assert.throws(() => formatDrillFailureManifestAggregateSummary({
    ...aggregate,
    classifications: { "typo-runtime": 1 },
    owners: { "drill-or-runtime": 1 },
    failures: [{
      ...aggregate.failures[0],
      owner: "drill-or-runtime",
      classification: "typo-runtime",
    }],
    nextActions: [{
      owner: "drill-or-runtime",
      classification: "typo-runtime",
      nextAction: aggregate.failures[0].nextAction,
      count: 1,
    }],
  }), /failures\[0\] has unknown classification "typo-runtime"/)
  assert.throws(() => formatDrillFailureManifestAggregateSummary({
    ...aggregate,
    runtimeSignals: { "lease-health": 2 },
  }), /runtimeSignals do not match failures/)
  assert.throws(() => formatDrillFailureManifestAggregateSummary({
    ...aggregate,
    runtimeSignalOwners: { "runtime-network": 1 },
  }), /runtimeSignalOwners do not match runtimeSignals/)
  assert.throws(() => formatDrillFailureManifestAggregateSummary({
    ...aggregate,
    owners: { "runtime-state": 1 },
    failures: [{
      ...aggregate.failures[0],
      owner: "runtime-state",
    }],
    nextActions: [{
      owner: "runtime-state",
      classification: aggregate.failures[0].classification,
      nextAction: aggregate.failures[0].nextAction,
      count: 1,
    }],
  }), /owner "runtime-state" does not match classification "provider-auth"/)
  assert.throws(() => formatDrillFailureManifestAggregateSummary({
    ...aggregate,
    failures: [{
      ...aggregate.failures[0],
      nextAction: "try something else",
    }],
    nextActions: [{
      owner: aggregate.failures[0].owner,
      classification: aggregate.failures[0].classification,
      nextAction: "try something else",
      count: 1,
    }],
  }), /failures\[0\] nextAction does not match classification "provider-auth"/)
})

function validManifest(overrides = {}) {
  return {
    schema: "arroba.drill.failure.v1",
    rootDir: "/tmp/arroba-drill",
    failedAt: "2026-06-13T00:00:00.000Z",
    metadata: { drill: "test-drill" },
    error: {
      name: "Error",
      message: "failed",
      stack: null,
    },
    ...overrides,
  }
}
