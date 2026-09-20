import assert from "node:assert/strict"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  combineBrowserComputerAbortSignals,
  runManagedBrowserComputerParityLive,
} from "./live-managed-browser-computer-parity-drill.mjs"

const OSS_SHA = "1".repeat(40)
const CLOUD_SHA = "2".repeat(40)
const IMAGE_DIGEST = `sha256:${"3".repeat(64)}`
const EVIDENCE_ROOT = path.join(os.tmpdir(), "chariox-live-m0-guard-test")

function config() {
  return {
    runId: "cha-16-managed-parity-live-guard-test",
    ossSha: OSS_SHA,
    cloudSha: CLOUD_SHA,
    image: {
      digest: IMAGE_DIGEST,
      signature: Buffer.alloc(64, 7).toString("base64"),
      signerFingerprint: `sha256:${"4".repeat(64)}`,
    },
    expected: {
      kernelId: "kernel-managed-1",
      machineId: "machine-managed-1",
      roomId: "room-managed-1",
      environmentId: "environment-managed-1",
    },
    resourceCeilings: {
      maximumRssBytes: 2_000_000_000,
      maximumCpuPercent: 300,
      minimumFreeMemoryBytes: 1_000_000_000,
      minimumFreeDiskBytes: 10_000_000_000,
      maximumHeartbeatAgeMs: 15_000,
      maximumPostRunRssDeltaBytes: 50_000_000,
      maximumPostRunDiskDeltaBytes: 10_000_000,
    },
    browserComputerGuard: {
      caps: { diskBytes: 10, memoryBytes: 20, processCount: 4, logBytes: 5 },
      preflight: { requiredMemoryBytes: 0, requiredDiskBytes: 0 },
      watchdogIntervalMs: 1,
      dockerPreconditions: persistenceDeclarations(),
    },
  }
}

function preflight() {
  return {
    image: {
      digest: IMAGE_DIGEST,
      signature: Buffer.alloc(64, 7).toString("base64"),
      signerFingerprint: `sha256:${"4".repeat(64)}`,
      verified: true,
    },
    source: { ossSha: OSS_SHA, cloudSha: CLOUD_SHA },
    protocol: { kernel: 322, relay: 18, relayVersion: "chariox-relay 0.1.0" },
    target: {
      kernelId: "kernel-managed-1",
      machineId: "machine-managed-1",
      heartbeatAgeMs: 500,
    },
    capabilities: {
      providers: { codex: "official", opencode: "official", claude: "official" },
      gitAuth: true,
      syntheticVault: true,
      browserStructuredActions: true,
      computerScreenshotInput: true,
      actorTakeover: true,
      persistence: true,
      selkies: true,
      novncRollback: true,
    },
    resources: {
      rssBytes: 200_000_000,
      cpuPercent: 5,
      freeMemoryBytes: 8_000_000_000,
      freeDiskBytes: 100_000_000_000,
    },
  }
}

function target(displayBackend) {
  return {
    kernelId: "kernel-managed-1",
    machineId: "machine-managed-1",
    roomId: "room-managed-1",
    environmentId: "environment-managed-1",
    displayBackend,
    roomCount: 1,
    browserCount: 1,
    profileCount: 1,
  }
}

function cleanInventory() {
  return {
    managedMachines: 0,
    rooms: 0,
    environments: 0,
    processes: 0,
    listeners: 0,
    containers: 0,
    profiles: 0,
    activeTargets: 0,
    temporaryFiles: 0,
    retainedEvidenceLeakCount: 0,
    resources: { rssDeltaBytes: 1_000_000, diskDeltaBytes: 1_000 },
  }
}

function transport() {
  const calls = []
  return {
    calls,
    resourceScope: "managed",
    async describePersistenceMutations() {
      return persistencePlan()
    },
    async run(step, input, options = {}) {
      calls.push({ step, input })
      await new Promise((resolve) => setTimeout(resolve, 2))
      if (step === "preflight") return preflight()
      if (step === "selkies.create") return target("selkies")
      if (step === "novnc.create") return target("novnc")
      if (step.endsWith(".attach")) return { ...target(input.displayBackend), client: input.client }
      const binding = target(input.displayBackend)
      if (step.endsWith(".providers")) return { ...binding, providers: { codex: "official", opencode: "official", claude: "official" }, providerStateCopied: false }
      if (step.endsWith(".browser")) return { ...binding, structuredActions: true, mutationCount: 1, browserCount: 1 }
      if (step.endsWith(".computer")) return { ...binding, screenshot: true, pointer: true, keyboard: true }
      if (step.endsWith(".takeover")) return { ...binding, overlayVisible: true, takeoverCompleted: true, actorAttributed: true }
      if (step.endsWith(".persistence")) {
        const evidence = persistenceEvidence()
        for (const mutation of evidence.persistenceMutations) {
          await options.onPersistenceMutation?.({ phase: "before", mutation })
          await options.onPersistenceMutation?.({ phase: "after", mutation })
        }
        return {
          ...binding,
          saved: true,
          restarted: true,
          sameRoom: true,
          sameEnvironment: true,
          sameProfile: true,
          ...evidence,
        }
      }
      if (step.endsWith(".vault")) return {
        ...binding,
        syntheticValueInserted: true,
        valueObservedOnlyAtTarget: true,
        leakScan: { arguments: 0, logs: 0, evidence: 0, prompts: 0, fixtures: 0 },
      }
      if (step.endsWith(".git")) return { ...binding, available: true, source: "product-managed" }
      if (step.endsWith(".reconnect")) return { ...binding, faultInjected: true, reconnected: true, duplicateActions: 0, duplicateBrowsers: 0 }
      if (step.endsWith(".destroy")) return { ...binding, destroyed: true }
      if (step === "novnc.rollback") return { ...binding, rollbackReachable: true, finalAcceptance: false }
      if (step === "cleanup.perform") return { attempted: true }
      if (step === "cleanup.inspect") return cleanInventory()
      throw new Error(`unexpected step ${step}`)
    },
  }
}

function sample(phase, overrun = false) {
  const values = {
    before: { diskAvailableBytes: 90, memoryAvailableBytes: 90, processCount: 2, logBytes: 0 },
    during: overrun
      ? { diskAvailableBytes: 70, memoryAvailableBytes: 60, processCount: 6, logBytes: 10 }
      : { diskAvailableBytes: 90, memoryAvailableBytes: 90, processCount: 2, logBytes: 2 },
    watchdog: overrun
      ? { diskAvailableBytes: 70, memoryAvailableBytes: 60, processCount: 6, logBytes: 10 }
      : { diskAvailableBytes: 90, memoryAvailableBytes: 90, processCount: 2, logBytes: 2 },
    after: { diskAvailableBytes: 95, memoryAvailableBytes: 95, processCount: 2, logBytes: 3 },
  }[phase]
  return {
    phase,
    memory: { totalBytes: 100, availableBytes: values.memoryAvailableBytes },
    disk: { totalBytes: 100, availableBytes: values.diskAvailableBytes },
    process: { count: values.processCount },
    logs: { bytes: values.logBytes },
    docker: { containers: [], volumes: [], images: [] },
    telemetry: {
      scope: "managed-target",
      authoritative: true,
      source: "managed-target-test",
      targetId: "machine-managed-1",
    },
  }
}

function persistenceInventory() {
  return {
    docker: {
      containers: ["chariox-slice-owned"],
      volumes: ["chariox-slice-owned-home"],
      networks: ["chariox-slice-owned-net"],
      images: ["chariox/browser:fixture"],
    },
  }
}

function persistenceEvidence() {
  const saveArgv = ["docker", "save", "chariox/browser:fixture", "-o", "/tmp/browser-state.tar"]
  const removeArgv = ["docker", "rm", "chariox-slice-owned"]
  const restoreArgv = ["docker", "load", "-i", "/tmp/browser-state.tar"]
  const saveReceipt = { ok: true, id: "save-receipt-1", archivePath: "/tmp/browser-state.tar" }
  const removeReceipt = { ok: true, id: "remove-receipt-1", parentReceiptId: saveReceipt.id }
  return {
    persistenceMutations: [
      {
        action: "save",
        argv: saveArgv,
        request: { action: "save", argv: saveArgv },
        before: persistenceInventory(),
        receipt: saveReceipt,
        checkpoints: { before: "before-docker-save", after: "after-docker-save" },
      },
      {
        action: "remove",
        argv: removeArgv,
        request: { action: "remove", argv: removeArgv },
        before: persistenceInventory(),
        saveReceipt,
        receipt: removeReceipt,
        checkpoints: { before: "before-docker-remove", after: "after-docker-remove" },
      },
      {
        action: "restore",
        argv: restoreArgv,
        request: { action: "restore", argv: restoreArgv },
        before: persistenceInventory(),
        saveReceipt,
        removeReceipt,
        receipt: { ok: true, id: "restore-receipt-1", parentReceiptId: removeReceipt.id, archivePath: "/tmp/browser-state.tar" },
        checkpoints: { before: "before-docker-restore", after: "after-docker-restore" },
      },
    ],
  }
}

function persistencePlan() {
  return {
    persistenceMutations: persistenceEvidence().persistenceMutations.map(({
      action,
      argv,
      request,
      checkpoints,
    }) => ({ action, argv, request, checkpoints })),
  }
}

function persistenceDeclarations() {
  const before = persistenceInventory()
  return [
    {
      action: "save",
      before,
      imageRef: "chariox/browser:fixture",
      savePath: "/tmp/browser-state.tar",
      command: ["docker", "save", "chariox/browser:fixture", "-o", "/tmp/browser-state.tar"],
    },
    {
      action: "remove",
      before,
      ownedContainers: ["chariox-slice-owned"],
      targetContainers: ["chariox-slice-owned"],
      saved: true,
      command: ["docker", "rm", "chariox-slice-owned"],
    },
    {
      action: "restore",
      before,
      imageRef: "chariox/browser:fixture",
      saved: true,
      removed: true,
      restorePath: "/tmp/browser-state.tar",
      command: [
        "docker", "load", "-i", "/tmp/browser-state.tar",
      ],
    },
  ]
}

test("live M0 watchdog aborts an executing resource overrun before unowned work can continue", async () => {
  const injected = transport()
  const report = await runManagedBrowserComputerParityLive({
    config: config(),
    transport: injected,
    evidenceRoot: EVIDENCE_ROOT,
    collectResourceSnapshot: ({ phase }) => sample(phase, true),
  })

  assert.equal(report.status, "failed")
  assert.equal(report.failure.code, "browser_computer_watchdog_failed")
  assert.equal(report.browserComputerGuard.resourceEvaluation.ok, false)
  assert.equal(report.browserComputerGuard.resourcePreflight.ok, true)
  assert.equal(report.browserComputerGuard.resourceSamples.map(({ phase }) => phase).join(","), "before,after")
  assert.equal(report.browserComputerGuard.watchdogSamples.length > 0, true)
  assert.equal(injected.calls.filter(({ step }) => step === "cleanup.inspect").length, 1)
})

test("live M0 watchdog bounds a hung telemetry probe and still completes owned cleanup", async () => {
  const watchdogConfig = config()
  watchdogConfig.browserComputerGuard.watchdogProbeTimeoutMs = 5
  const injected = transport()
  let probeAborted = false
  const report = await runManagedBrowserComputerParityLive({
    config: watchdogConfig,
    transport: injected,
    evidenceRoot: EVIDENCE_ROOT,
    collectResourceSnapshot: ({ phase, signal }) => {
      if (phase === "watchdog") {
        return new Promise(() => {
          signal.addEventListener("abort", () => {
            probeAborted = true
          }, { once: true })
        })
      }
      return sample(phase)
    },
  })

  assert.equal(report.status, "failed")
  assert.equal(report.failure.code, "browser_computer_watchdog_sampling_failed")
  assert.equal(report.browserComputerGuard.watchdog.samplingFailures.length, 1)
  assert.equal(report.browserComputerGuard.watchdog.samplingFailures[0].code, "browser_computer_watchdog_sampling_timeout")
  assert.equal(probeAborted, true)
  assert.equal(injected.calls.filter(({ step }) => step === "cleanup.perform").length, 1)
  assert.equal(injected.calls.filter(({ step }) => step === "cleanup.inspect").length, 1)
})

test("live M0 entry executes a fault checkpoint and fails the run while cleanup still executes", async () => {
  const injected = transport()
  const report = await runManagedBrowserComputerParityLive({
    config: config(),
    transport: injected,
    evidenceRoot: EVIDENCE_ROOT,
    faultAt: "during-browser",
    collectResourceSnapshot: ({ phase }) => sample(phase),
  })

  assert.equal(report.status, "failed")
  assert.equal(report.browserComputerGuard.fault.status, "failed")
  assert.equal(report.browserComputerGuard.resourcePreflight.ok, true)
  assert.deepEqual(report.browserComputerGuard.fault.faultCheckpoint, {
    requested: "during-browser",
    reached: 1,
    exercised: true,
  })
  assert.equal(injected.calls.some(({ step }) => step === "cleanup.perform"), true)
  assert.equal(injected.calls.some(({ step }) => step === "cleanup.inspect"), true)
})

test("runbook-shaped ceilings remain runnable without a duplicate guard cap declaration", async () => {
  const runbookConfig = config()
  delete runbookConfig.browserComputerGuard.caps
  const report = await runManagedBrowserComputerParityLive({
    config: runbookConfig,
    transport: transport(),
    evidenceRoot: EVIDENCE_ROOT,
    collectResourceSnapshot: ({ phase }) => sample(phase),
  })
  assert.equal(report.status, "passed", report.failure?.code ?? "runbook-shaped config did not pass")
  assert.equal(report.browserComputerGuard.resourceEvaluation.ok, true)
  assert.equal(report.browserComputerGuard.resourceEvaluation.caps.memoryBytes, 2_000_000_000)
})

test("remote live entry fails closed when managed-target telemetry is absent", async () => {
  const injected = transport()
  const report = await runManagedBrowserComputerParityLive({
    config: config(),
    transport: injected,
    evidenceRoot: EVIDENCE_ROOT,
    collectResourceSnapshot: ({ phase }) => {
      const value = sample(phase)
      delete value.telemetry
      return value
    },
  })
  assert.equal(report.status, "failed")
  assert.match(report.failure.message, /managed-target telemetry/)
  assert.equal(injected.calls.some(({ step }) => step === "preflight"), false)
})

test("remote live entry fails closed when managed telemetry belongs to another target", async () => {
  const injected = transport()
  const report = await runManagedBrowserComputerParityLive({
    config: config(),
    transport: injected,
    evidenceRoot: EVIDENCE_ROOT,
    collectResourceSnapshot: ({ phase }) => ({
      ...sample(phase),
      telemetry: {
        scope: "managed-target",
        authoritative: true,
        source: "foreign-managed-target",
        targetId: "machine-foreign-1",
      },
    }),
  })
  assert.equal(report.status, "failed")
  assert.match(report.failure.message, /does not match an expected machine/)
  assert.equal(injected.calls.some(({ step }) => step === "preflight"), false)
})

test("live transport preserves caller cancellation while combining workload cancellation", () => {
  const caller = new AbortController()
  const workload = new AbortController()
  assert.equal(combineBrowserComputerAbortSignals(caller.signal), caller.signal)
  const combined = combineBrowserComputerAbortSignals(caller.signal, workload.signal)
  assert.notEqual(combined, caller.signal)
  caller.abort(new Error("caller cancelled"))
  assert.equal(combined.aborted, true)
  assert.equal(combined.reason.message, "caller cancelled")

  const secondCaller = new AbortController()
  const secondWorkload = new AbortController()
  const secondCombined = combineBrowserComputerAbortSignals(secondCaller.signal, secondWorkload.signal)
  secondWorkload.abort(new Error("watchdog cancelled"))
  assert.equal(secondCombined.aborted, true)
  assert.equal(secondCombined.reason.message, "watchdog cancelled")
})

test("explicitly local transport may use only an explicitly marked local fallback", async () => {
  const localConfig = config()
  localConfig.browserComputerGuard.resourceTelemetry = { mode: "local", allowLocalFallback: true }
  const localTransport = transport()
  localTransport.resourceScope = "local"
  const report = await runManagedBrowserComputerParityLive({
    config: localConfig,
    transport: localTransport,
    evidenceRoot: EVIDENCE_ROOT,
    collectResourceSnapshot: ({ phase }) => ({
      ...sample(phase),
      telemetry: {
        scope: "local-host",
        authoritative: true,
        fallback: true,
        source: "explicit-local-test",
      },
    }),
  })
  assert.equal(report.status, "passed")
  assert.equal(report.browserComputerGuard.watchdog.telemetryMode, "local-host")
})
