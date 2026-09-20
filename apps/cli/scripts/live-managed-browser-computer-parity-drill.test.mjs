import assert from "node:assert/strict"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { runManagedBrowserComputerParityLive } from "./live-managed-browser-computer-parity-drill.mjs"

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
    async run(step, input) {
      calls.push({ step, input })
      if (step === "preflight") return preflight()
      if (step === "selkies.create") return target("selkies")
      if (step === "novnc.create") return target("novnc")
      if (step.endsWith(".attach")) return { ...target(input.displayBackend), client: input.client }
      const binding = target(input.displayBackend)
      if (step.endsWith(".providers")) return { ...binding, providers: { codex: "official", opencode: "official", claude: "official" }, providerStateCopied: false }
      if (step.endsWith(".browser")) return { ...binding, structuredActions: true, mutationCount: 1, browserCount: 1 }
      if (step.endsWith(".computer")) return { ...binding, screenshot: true, pointer: true, keyboard: true }
      if (step.endsWith(".takeover")) return { ...binding, overlayVisible: true, takeoverCompleted: true, actorAttributed: true }
      if (step.endsWith(".persistence")) return { ...binding, saved: true, restarted: true, sameRoom: true, sameEnvironment: true, sameProfile: true }
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
    after: { diskAvailableBytes: 95, memoryAvailableBytes: 95, processCount: 2, logBytes: 3 },
  }[phase]
  return {
    phase,
    memory: { totalBytes: 100, availableBytes: values.memoryAvailableBytes },
    disk: { totalBytes: 100, availableBytes: values.diskAvailableBytes },
    process: { count: values.processCount },
    logs: { bytes: values.logBytes },
    docker: { containers: [], volumes: [], images: [] },
  }
}

test("live M0 entry fails an executing resource overrun after the real harness returns", async () => {
  const injected = transport()
  const report = await runManagedBrowserComputerParityLive({
    config: config(),
    transport: injected,
    evidenceRoot: EVIDENCE_ROOT,
    collectResourceSnapshot: ({ phase }) => sample(phase, true),
  })

  assert.equal(report.status, "failed")
  assert.equal(report.failure.code, "browser_computer_resource_cap_failed")
  assert.equal(report.browserComputerGuard.resourceEvaluation.ok, false)
  assert.equal(report.browserComputerGuard.resourcePreflight.ok, true)
  assert.equal(report.browserComputerGuard.resourceSamples.map(({ phase }) => phase).join(","), "before,during,after")
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
