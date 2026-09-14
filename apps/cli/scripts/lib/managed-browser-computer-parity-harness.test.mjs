import assert from "node:assert/strict"
import { test } from "node:test"

import {
  MANAGED_BROWSER_COMPUTER_PARITY_SCHEMA,
  runManagedBrowserComputerParityHarness,
} from "./managed-browser-computer-parity-harness.mjs"

const OSS_SHA = "1".repeat(40)
const CLOUD_SHA = "2".repeat(40)
const IMAGE_DIGEST = `sha256:${"3".repeat(64)}`

function config(overrides = {}) {
  return {
    runId: "cha-16-managed-parity-001",
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
    ...overrides,
  }
}

function preflight(overrides = {}) {
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
    ...overrides,
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

function cleanInventory(overrides = {}) {
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
    resources: {
      rssDeltaBytes: 1_000_000,
      diskDeltaBytes: 1_000,
    },
    ...overrides,
  }
}

function successfulResult(step, input) {
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
  if (step.endsWith(".vault")) return { ...binding,
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
}

function transport({ mutate = {}, failStep = null } = {}) {
  const calls = []
  return {
    calls,
    async run(step, input) {
      calls.push({ step, input })
      if (step === failStep) throw new Error("injected partial failure")
      const result = successfulResult(step, input)
      return mutate[step] ? mutate[step](result) : result
    },
  }
}

test("managed parity harness accepts Selkies only after all released Web and TUI paths pass", async () => {
  const injected = transport()
  const report = await runManagedBrowserComputerParityHarness({ config: config(), transport: injected })

  assert.equal(report.schema, MANAGED_BROWSER_COMPUTER_PARITY_SCHEMA)
  assert.equal(report.status, "passed")
  assert.equal(report.acceptanceBackend, "selkies")
  assert.equal(report.rollback.status, "rollback_only")
  assert.deepEqual(report.clients, ["web", "local_tui", "remote_tui"])
  assert.deepEqual(report.providers, ["codex", "opencode", "claude"])
  assert.equal(report.cleanup.clean, true)
  const preflightRequest = injected.calls.find(({ step }) => step === "preflight").input
  assert.equal(Object.hasOwn(preflightRequest, "binding"), false)
  assert.equal(Object.hasOwn(preflightRequest, "image"), false)
  assert.equal(Object.hasOwn(preflightRequest, "source"), false)
  assert.deepEqual(injected.calls.filter(({ step }) => step.endsWith(".attach")).map(({ input }) => input.client), [
    "web", "local_tui", "remote_tui",
    "web", "local_tui", "remote_tui",
  ])
})

test("managed parity harness rejects a run id that could escape its evidence root", async () => {
  await assert.rejects(
    () => runManagedBrowserComputerParityHarness({ config: config({ runId: "../../outside" }), transport: transport() }),
    /run id/,
  )
})

for (const protocol of [321, null]) {
  test(`managed parity harness rejects ${protocol ?? "unknown"} protocol before environment creation`, async () => {
    const injected = transport({ mutate: { preflight: (value) => ({ ...value, protocol: { ...value.protocol, kernel: protocol } }) } })
    const report = await runManagedBrowserComputerParityHarness({ config: config(), transport: injected })
    assert.equal(report.status, "failed")
    assert.equal(report.failure.code, "kernel_protocol_322_required")
    assert.equal(injected.calls.some(({ step }) => step.endsWith(".create")), false)
  })
}

test("managed parity harness rejects an unsigned or unverified image", async () => {
  for (const image of [
    { ...preflight().image, signature: "" },
    { ...preflight().image, verified: false },
  ]) {
    const injected = transport({ mutate: { preflight: (value) => ({ ...value, image }) } })
    const report = await runManagedBrowserComputerParityHarness({ config: config(), transport: injected })
    assert.equal(report.failure.code, "signed_image_required")
    assert.equal(injected.calls.some(({ step }) => step.endsWith(".create")), false)
  }
})

test("managed parity harness rejects a stale target heartbeat", async () => {
  const injected = transport({ mutate: { preflight: (value) => ({ ...value, target: { ...value.target, heartbeatAgeMs: 15_001 } }) } })
  const report = await runManagedBrowserComputerParityHarness({ config: config(), transport: injected })
  assert.equal(report.failure.code, "fresh_target_required")
})

test("managed parity harness rejects wrong preflight and rebound kernel or machine identity", async () => {
  for (const mutate of [
    { preflight: (value) => ({ ...value, target: { ...value.target, kernelId: "wrong" } }) },
    { preflight: (value) => ({ ...value, target: { ...value.target, machineId: "wrong" } }) },
    { "selkies.browser": (value) => ({ ...value, kernelId: "wrong", machineId: "machine-managed-1" }) },
  ]) {
    const injected = transport({ mutate })
    const report = await runManagedBrowserComputerParityHarness({ config: config(), transport: injected })
    assert.equal(report.failure.code, "target_identity_mismatch")
  }
})

test("managed parity harness requires exact relay protocol and released version evidence", async () => {
  for (const protocol of [
    { kernel: 322, relay: null, relayVersion: "chariox-relay 0.1.0" },
    { kernel: 322, relay: 18, relayVersion: "" },
  ]) {
    const injected = transport({ mutate: { preflight: (value) => ({ ...value, protocol }) } })
    const report = await runManagedBrowserComputerParityHarness({ config: config(), transport: injected })
    assert.equal(report.failure.code, "relay_version_required")
  }
})

test("managed parity harness requires every official provider plus Git and synthetic vault capabilities", async () => {
  for (const [field, expectedCode] of [
    ["codex", "official_provider_capability_required"],
    ["opencode", "official_provider_capability_required"],
    ["claude", "official_provider_capability_required"],
    ["gitAuth", "git_auth_capability_required"],
    ["syntheticVault", "synthetic_vault_capability_required"],
  ]) {
    const injected = transport({ mutate: {
      preflight: (value) => field in value.capabilities.providers
        ? { ...value, capabilities: { ...value.capabilities, providers: { ...value.capabilities.providers, [field]: "missing" } } }
        : { ...value, capabilities: { ...value.capabilities, [field]: false } },
    } })
    const report = await runManagedBrowserComputerParityHarness({ config: config(), transport: injected })
    assert.equal(report.failure.code, expectedCode)
  }
})

test("managed parity harness rejects provider evidence that does not prove zero state copying", async () => {
  const injected = transport({ mutate: {
    "selkies.providers": (value) => ({ ...value, providerStateCopied: true }),
  } })
  const report = await runManagedBrowserComputerParityHarness({ config: config(), transport: injected })
  assert.equal(report.failure.code, "provider_state_copy_forbidden")
})

test("managed parity harness rejects duplicate Rooms, browsers, and profiles", async () => {
  for (const field of ["roomCount", "browserCount", "profileCount"]) {
    const injected = transport({ mutate: { "selkies.create": (value) => ({ ...value, [field]: 2 }) } })
    const report = await runManagedBrowserComputerParityHarness({ config: config(), transport: injected })
    assert.equal(report.failure.code, "single_room_environment_required")
  }
})

test("noVNC rollback can never claim final acceptance", async () => {
  const injected = transport({ mutate: { "novnc.rollback": (value) => ({ ...value, finalAcceptance: true }) } })
  const report = await runManagedBrowserComputerParityHarness({ config: config(), transport: injected })
  assert.equal(report.status, "failed")
  assert.equal(report.failure.code, "novnc_cannot_claim_acceptance")
  assert.notEqual(report.acceptanceBackend, "novnc")
})

test("partial failure still runs cleanup exactly once and cannot report success", async () => {
  const injected = transport({ failStep: "selkies.computer" })
  const report = await runManagedBrowserComputerParityHarness({ config: config(), transport: injected })
  assert.equal(report.status, "failed")
  assert.equal(report.failure.code, "managed_parity_step_failed")
  assert.equal(injected.calls.filter(({ step }) => step === "cleanup.perform").length, 1)
  assert.equal(injected.calls.filter(({ step }) => step === "cleanup.inspect").length, 1)
  assert.equal(injected.calls.some(({ step }) => step === "novnc.create"), false)
})

test("a stalled product step is bounded, aborted, and followed by cleanup", async () => {
  const injected = transport()
  const originalRun = injected.run
  let aborted = false
  injected.run = async (step, input, options) => {
    if (step !== "selkies.computer") return originalRun(step, input, options)
    options.signal.addEventListener("abort", () => { aborted = true }, { once: true })
    return new Promise(() => {})
  }
  const report = await runManagedBrowserComputerParityHarness({
    config: config({ stepTimeoutMs: 5 }),
    transport: injected,
  })
  assert.equal(report.failure.code, "managed_parity_step_timeout")
  assert.equal(aborted, true)
  assert.equal(injected.calls.filter(({ step }) => step === "cleanup.perform").length, 1)
})

test("operator interruption aborts the active step but still completes cleanup", async () => {
  const controller = new AbortController()
  const injected = transport()
  const originalRun = injected.run
  injected.run = async (step, input, options) => {
    if (step !== "selkies.computer") return originalRun(step, input, options)
    queueMicrotask(() => controller.abort())
    return new Promise((resolve, reject) => {
      options.signal.addEventListener("abort", () => reject(new Error("transport aborted")), { once: true })
    })
  }
  const report = await runManagedBrowserComputerParityHarness({
    config: config(),
    transport: injected,
    signal: controller.signal,
  })
  assert.equal(report.failure.code, "managed_parity_interrupted")
  assert.equal(injected.calls.filter(({ step }) => step === "cleanup.perform").length, 1)
  assert.equal(report.cleanup.clean, true)
})

test("secret-looking transport output is never retained in evidence", async () => {
  const injected = transport({ mutate: {
    "selkies.browser": (value) => ({ ...value, diagnostic: `Bearer ${"a".repeat(32)}` }),
  } })
  const report = await runManagedBrowserComputerParityHarness({ config: config(), transport: injected })
  assert.equal(report.status, "failed")
  assert.equal(report.failure.code, "managed_parity_step_failed")
  assert.doesNotMatch(JSON.stringify(report), /Bearer/)
})

test("cleanup command success cannot hide residual resources or leak evidence", async () => {
  for (const residue of [
    { processes: 1 },
    { listeners: 1 },
    { containers: 1 },
    { profiles: 1 },
    { retainedEvidenceLeakCount: 1 },
    { resources: { rssDeltaBytes: 50_000_001, diskDeltaBytes: 1_000 } },
  ]) {
    const injected = transport({ mutate: { "cleanup.inspect": (value) => ({ ...value, ...residue }) } })
    const report = await runManagedBrowserComputerParityHarness({ config: config(), transport: injected })
    assert.equal(report.status, "failed")
    assert.equal(report.failure.code, "cleanup_incomplete")
    assert.equal(report.cleanup.clean, false)
  }
})
