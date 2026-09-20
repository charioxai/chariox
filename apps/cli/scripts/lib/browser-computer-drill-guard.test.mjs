import assert from "node:assert/strict"
import { mkdtemp, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import {
  assertBrowserComputerDockerPreconditions,
  assertBrowserComputerEvidencePath,
  assertBrowserComputerPreflight,
  assertBrowserComputerResourceCaps,
  assertSecretSafeBrowserComputerEvidence,
  collectBrowserComputerResourceSnapshot,
  defaultBrowserComputerEvidenceDir,
  evaluateBrowserComputerCleanup,
  evaluateBrowserComputerDockerPreconditions,
  evaluateBrowserComputerPreflight,
  evaluateBrowserComputerResourceCaps,
  normalizeBrowserComputerCaps,
  parseBrowserComputerByteBudget,
  redactBrowserComputerEvidence,
  runBrowserComputerFaultGuard,
  serializeBrowserComputerEvidence,
} from "./browser-computer-drill-guard.mjs"

test("browser/computer evidence defaults outside repositories", () => {
  assert.equal(
    defaultBrowserComputerEvidenceDir("run-1", "/Users/tester"),
    "/Users/tester/.codex/evidence/browser-computer-use/m0/run-1",
  )
  assert.equal(
    assertBrowserComputerEvidencePath("/Users/tester/.codex/evidence/run-1", ["/work/chariox"]),
    "/Users/tester/.codex/evidence/run-1",
  )
  assert.throws(
    () => assertBrowserComputerEvidencePath("/work/chariox/.artifacts/run-1", ["/work/chariox"]),
    /must stay outside repositories/,
  )
  assert.throws(() => defaultBrowserComputerEvidenceDir(""), /run id is required/)
})

test("resource collection is deterministic and uses injected Docker/macOS probes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-browser-resource-test-"))
  const commands = []
  try {
    const result = await collectBrowserComputerResourceSnapshot({
      filesystemPath: rootDir,
      platform: "darwin",
      phase: "before",
      sampleId: "sample-0",
      now: () => "2026-09-20T00:00:00.000Z",
      processCount: 3,
      logBytes: 7,
      runCommand: async (command, args) => {
        commands.push([command, ...args])
        if (command === "vm_stat") {
          return {
            code: 0,
            stdout: [
              "Mach Virtual Memory Statistics: (page size of 16384 bytes)",
              "Pages free: 10.",
              "Pages inactive: 20.",
              "Pages speculative: 5.",
              "Pages purgeable: 2.",
            ].join("\n"),
            stderr: "",
          }
        }
        if (args[0] === "ps") return { code: 0, stdout: "chariox-slice-existing\nunrelated\n", stderr: "" }
        if (args[0] === "volume") return { code: 0, stdout: "chariox-slice-existing-home\n", stderr: "" }
        throw new Error(`unexpected command: ${command} ${args.join(" ")}`)
      },
    })

    assert.equal(result.capturedAt, "2026-09-20T00:00:00.000Z")
    assert.equal(result.phase, "before")
    assert.equal(result.sampleId, "sample-0")
    assert.equal(result.memory.availableBytes, 37 * 16_384)
    assert.deepEqual(result.process, { count: 3 })
    assert.deepEqual(result.logs, { bytes: 7 })
    assert.deepEqual(result.docker.containers, ["chariox-slice-existing", "unrelated"])
    assert.deepEqual(result.docker.volumes, ["chariox-slice-existing-home"])
    assert.deepEqual(commands.map((command) => command.slice(0, 2)), [
      ["docker", "ps"],
      ["docker", "volume"],
      ["vm_stat"],
    ])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("resource preflight compares byte budgets and limits concurrent slices", () => {
  const result = evaluateBrowserComputerPreflight(snapshot({
    memoryAvailableBytes: 19,
    diskAvailableBytes: 14,
    containers: ["chariox-slice-existing"],
  }), { requiredMemoryBytes: 20, requiredDiskBytes: 15 })

  assert.equal(result.ok, false)
  assert.deepEqual(result.existingSliceContainers, ["chariox-slice-existing"])
  assert.match(result.violations.join("\n"), /available memory 19 bytes.*20 bytes/)
  assert.match(result.violations.join("\n"), /available disk 14 bytes.*15 bytes/)
  assert.match(result.violations.join("\n"), /single-slice developer run unsafe/)
  assert.throws(
    () => assertBrowserComputerPreflight(snapshot({ memoryAvailableBytes: 19 }), { requiredMemoryBytes: 20 }),
    /resource preflight failed/,
  )
})

test("resource caps pass with exact before/during/after samples", () => {
  const caps = normalizeBrowserComputerCaps({ diskBytes: 10, memoryBytes: 10, processCount: 3, logBytes: 4 })
  const result = assertBrowserComputerResourceCaps(resourceSamples({
    during: { diskAvailableBytes: 90, memoryAvailableBytes: 90, processCount: 3, logBytes: 4 },
    after: { diskAvailableBytes: 95, memoryAvailableBytes: 95, processCount: 2, logBytes: 3 },
  }), caps)

  assert.equal(result.ok, true)
  assert.deepEqual(result.metrics, {
    diskGrowthBytes: 10,
    peakMemoryBytes: 10,
    peakProcessCount: 3,
    logGrowthBytes: 4,
    peakLogBytes: 4,
  })
})

test("resource caps fail closed for declared disk/memory/process/log overruns", () => {
  const result = evaluateBrowserComputerResourceCaps(resourceSamples({
    during: { diskAvailableBytes: 80, memoryAvailableBytes: 70, processCount: 5, logBytes: 10 },
  }), { diskBytes: 10, memoryBytes: 20, processCount: 4, logBytes: 5 })

  assert.equal(result.ok, false)
  assert.match(result.violations.join("\n"), /disk growth 20 bytes exceeds cap 10 bytes/)
  assert.match(result.violations.join("\n"), /peak memory 30 bytes exceeds cap 20 bytes/)
  assert.match(result.violations.join("\n"), /peak process count 5 exceeds cap 4/)
  assert.match(result.violations.join("\n"), /log growth 10 bytes exceeds cap 5 bytes/)
})

test("resource growth is measured against the before sample, not only the final sample", () => {
  const result = evaluateBrowserComputerResourceCaps(resourceSamples({
    during: { diskAvailableBytes: 96, memoryAvailableBytes: 96, processCount: 2, logBytes: 2 },
    after: { diskAvailableBytes: 80, memoryAvailableBytes: 98, processCount: 2, logBytes: 9 },
  }), { diskBytes: 10, memoryBytes: 5, processCount: 2, logBytes: 5 })

  assert.equal(result.ok, false)
  assert.equal(result.metrics.diskGrowthBytes, 20)
  assert.equal(result.metrics.logGrowthBytes, 9)
  assert.match(result.violations.join("\n"), /disk growth 20 bytes/)
  assert.match(result.violations.join("\n"), /log growth 9 bytes/)
})

test("resource evidence rejects a missing phase or missing process/log sample", () => {
  const incomplete = resourceSamples({}).slice(0, 2)
  const missingMetrics = resourceSamples({ after: { processCount: undefined, logBytes: undefined } })
  delete missingMetrics[2].process
  delete missingMetrics[2].logs

  const incompleteResult = evaluateBrowserComputerResourceCaps(incomplete, {
    diskBytes: 10, memoryBytes: 10, processCount: 3, logBytes: 4,
  })
  const missingResult = evaluateBrowserComputerResourceCaps(missingMetrics, {
    diskBytes: 10, memoryBytes: 10, processCount: 3, logBytes: 4,
  })
  assert.equal(incompleteResult.ok, false)
  assert.match(incompleteResult.violations.join("\n"), /exactly one before, during, and after/)
  assert.match(incompleteResult.violations.join("\n"), /missing the after sample/)
  assert.equal(missingResult.ok, false)
  assert.match(missingResult.violations.join("\n"), /after sample is missing a safe process count/)
  assert.match(missingResult.violations.join("\n"), /after sample is missing a safe log byte count/)
})

test("blank budgets remain observational while malformed caps are rejected", () => {
  for (const value of [undefined, "", " ", "\t\n"]) {
    const budget = parseBrowserComputerByteBudget(value)
    assert.equal(budget, undefined)
    assert.equal(evaluateBrowserComputerPreflight(snapshot(), {
      requiredMemoryBytes: budget,
      requiredDiskBytes: budget,
    }).ok, true)
  }
  assert.deepEqual(normalizeBrowserComputerCaps({
    maxDiskBytes: 1, maxMemoryBytes: 2, maxProcesses: 3, maxLogBytes: 4,
  }), { diskBytes: 1, memoryBytes: 2, processCount: 3, logBytes: 4 })
  assert.throws(() => normalizeBrowserComputerCaps({ diskBytes: 1 }), /memoryBytes is required/)
  assert.throws(() => normalizeBrowserComputerCaps({ diskBytes: -1, memoryBytes: 1, processCount: 1, logBytes: 1 }), /non-negative/)
})

test("cleanup accounts for owned resources and rejects residue", async () => {
  const tempRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-browser-cleanup-test-"))
  try {
    const result = await evaluateBrowserComputerCleanup({
      before: snapshot({ containers: ["chariox-slice-unrelated"], volumes: ["chariox-slice-unrelated-home"] }),
      after: snapshot({
        memoryAvailableBytes: 90,
        diskAvailableBytes: 80,
        containers: ["chariox-slice-unrelated", "chariox-slice-owned", "chariox-slice-leaked"],
        volumes: ["chariox-slice-unrelated-home", "chariox-slice-owned-home"],
      }),
      ownedContainers: ["chariox-slice-owned"],
      ownedVolumes: ["chariox-slice-owned-home"],
      tempRoots: [tempRoot],
      childProcesses: [
        { drillLabel: "kernel", exitCode: null, signalCode: null },
        { drillLabel: "fixture", exitCode: 0, signalCode: null },
      ],
    })

    assert.equal(result.ok, false)
    assert.match(result.violations.join("\n"), /owned container remains: chariox-slice-owned/)
    assert.match(result.violations.join("\n"), /new slice container remains: chariox-slice-leaked/)
    assert.match(result.violations.join("\n"), /owned volume remains: chariox-slice-owned-home/)
    assert.match(result.violations.join("\n"), /temporary root remains/)
    assert.match(result.violations.join("\n"), /child process remains alive: kernel/)
    assert.deepEqual(result.cleanupAccounting.remainingOwned, {
      containers: ["chariox-slice-owned"],
      volumes: ["chariox-slice-owned-home"],
    })
  } finally {
    await rm(tempRoot, { recursive: true, force: true })
  }
})

test("cleanup passes with exact owned action accounting and restored inventory", async () => {
  const removedRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-browser-cleanup-pass-"))
  await rm(removedRoot, { recursive: true, force: true })
  const result = await evaluateBrowserComputerCleanup({
    before: snapshot({
      memoryAvailableBytes: 70,
      diskAvailableBytes: 60,
      containers: ["chariox-slice-owned"],
      volumes: ["chariox-slice-owned-home"],
    }),
    after: snapshot({ memoryAvailableBytes: 75, diskAvailableBytes: 58 }),
    ownedContainers: ["chariox-slice-owned"],
    ownedVolumes: ["chariox-slice-owned-home"],
    tempRoots: [removedRoot],
    cleanupActions: [
      { kind: "container", name: "chariox-slice-owned", ok: true },
      { kind: "volume", name: "chariox-slice-owned-home", ok: true },
    ],
  })

  assert.equal(result.ok, true)
  assert.deepEqual(result.violations, [])
  assert.deepEqual(result.cleanupAccounting.removedOwned, {
    containers: ["chariox-slice-owned"],
    volumes: ["chariox-slice-owned-home"],
  })
  assert.equal(result.memoryAvailableDeltaBytes, 5)
  assert.equal(result.diskAvailableDeltaBytes, -2)
})

test("Docker save/remove/restore preconditions fail closed and reject broad prune", () => {
  const inventory = {
    memory: { totalBytes: 100, availableBytes: 80 },
    disk: { totalBytes: 100, availableBytes: 80 },
    docker: {
      containers: ["chariox-slice-owned"],
      volumes: ["chariox-slice-owned-home"],
      images: ["chariox/browser:fixture"],
    },
  }
  const save = evaluateBrowserComputerDockerPreconditions({
    action: "save",
    before: inventory,
    imageRef: "chariox/browser:fixture",
    savePath: "/tmp/browser-state.tar",
    command: ["docker", "save", "chariox/browser:fixture", "-o", "/tmp/browser-state.tar"],
  })
  assert.equal(save.ok, true)

  const broadRemove = evaluateBrowserComputerDockerPreconditions({
    action: "remove",
    before: inventory,
    ownedContainers: ["chariox-slice-owned"],
    targetContainers: ["chariox-slice-owned"],
    saved: true,
    command: ["docker", "system", "prune", "-af"],
  })
  assert.equal(broadRemove.ok, false)
  assert.match(broadRemove.violations.join("\n"), /broad prune/)
  assert.match(broadRemove.violations.join("\n"), /requires an exact rm command/)

  const missingRestoreReceipt = evaluateBrowserComputerDockerPreconditions({
    action: "restore",
    before: inventory,
    ownedContainers: ["chariox-slice-owned"],
    targetContainers: ["chariox-slice-owned"],
    saved: true,
    restorePath: "/tmp/browser-state.tar",
    command: ["docker", "load", "-i", "/tmp/browser-state.tar"],
  })
  assert.equal(missingRestoreReceipt.ok, false)
  assert.match(missingRestoreReceipt.violations.join("\n"), /successful exact remove receipt/)
  assert.throws(() => assertBrowserComputerDockerPreconditions({
    action: "remove",
    before: inventory,
    ownedContainers: ["chariox-slice-owned"],
    targetContainers: ["chariox-slice-owned"],
    command: ["docker", "rm", "chariox-slice-owned"],
  }), /preconditions failed/)
})

test("fault checkpoint interruption still runs owned cleanup", async () => {
  let cleanupCalls = 0
  const result = await runBrowserComputerFaultGuard({
    faultAt: "during-browser",
    now: () => "2026-09-20T00:00:00.000Z",
    operation: async ({ checkpoint }) => {
      await checkpoint("before-browser-start", { password: "do-not-record" })
      await checkpoint("during-browser")
    },
    cleanup: async ({ interrupted }) => {
      cleanupCalls += 1
      return { clean: true, interrupted }
    },
  })

  assert.equal(result.status, "failed")
  assert.equal(result.interrupted, true)
  assert.equal(cleanupCalls, 1)
  assert.deepEqual(result.checkpoints.map((entry) => entry.name), [
    "before-operation",
    "before-browser-start",
    "during-browser",
    "before-cleanup",
    "after-cleanup",
  ])
  assert.match(result.failure.message, /injected browser\/computer fault at during-browser/)
  assert.equal(JSON.stringify(result).includes("do-not-record"), false)
})

test("fault guard returns a deterministic pass with after-cleanup evidence", async () => {
  const result = await runBrowserComputerFaultGuard({
    now: () => "2026-09-20T00:00:00.000Z",
    operation: async ({ checkpoint }) => {
      await checkpoint("before-docker-save")
      await checkpoint("after-docker-save")
    },
    cleanup: async () => ({ clean: true, ownedResourcesRemoved: 2 }),
  })

  assert.equal(result.status, "passed")
  assert.equal(result.failure, null)
  assert.equal(result.checkpoints.at(-1).name, "after-cleanup")
  assert.equal(result.checkpoints.every((entry) => entry.capturedAt === "2026-09-20T00:00:00.000Z"), true)
})

test("evidence redaction removes secret values, secret-shaped fields, and control bytes", () => {
  const secret = "bearer-secret-123"
  const evidence = {
    authorization: `Bearer ${secret}`,
    nested: {
      token: secret,
      message: `request token=${secret} saw\u0000 control`,
    },
    ordinary: "visible",
  }
  const sanitized = assertSecretSafeBrowserComputerEvidence(evidence, { secretValues: [secret] })
  const serialized = serializeBrowserComputerEvidence(evidence, { secretValues: [secret] })
  assert.equal(sanitized.authorization, "<redacted>")
  assert.equal(sanitized.nested.token, "<redacted>")
  assert.equal(serialized.includes(secret), false)
  assert.equal(serialized.includes("\u0000"), false)
  assert.equal(redactBrowserComputerEvidence("x".repeat(20), { maxStringLength: 8 }).startsWith("<truncated>"), true)
})

function snapshot({
  memoryAvailableBytes = 100,
  diskAvailableBytes = 100,
  containers = [],
  volumes = [],
  processCount = 2,
  logBytes = 0,
} = {}) {
  return {
    memory: { totalBytes: 100, availableBytes: memoryAvailableBytes },
    disk: { totalBytes: 100, availableBytes: diskAvailableBytes },
    process: { count: processCount },
    logs: { bytes: logBytes },
    docker: { containers, volumes },
  }
}

function resourceSamples({ during = {}, after = {} } = {}) {
  return [
    { phase: "before", ...snapshot() },
    { phase: "during", ...snapshot(during) },
    { phase: "after", ...snapshot(after) },
  ]
}
