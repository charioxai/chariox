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
  assertBrowserComputerResourceTelemetry,
  assertSecretSafeBrowserComputerEvidence,
  collectBrowserComputerResourceSnapshot,
  defaultBrowserComputerEvidenceDir,
  deriveBrowserComputerCapsFromResourceCeilings,
  evaluateBrowserComputerCleanup,
  evaluateBrowserComputerDockerPreconditions,
  evaluateBrowserComputerPersistenceMutationSeams,
  evaluateBrowserComputerPreflight,
  evaluateBrowserComputerResourceCaps,
  evaluateBrowserComputerResourceTelemetry,
  evaluateBrowserComputerResourceWatchdogSample,
  normalizeBrowserComputerCaps,
  parseBrowserComputerByteBudget,
  parseBrowserComputerDockerMutationArgv,
  redactBrowserComputerEvidence,
  resolveBrowserComputerCaps,
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
        if (args[0] === "image") return { code: 0, stdout: "chariox/browser:fixture\n<none>:<none>\n", stderr: "" }
        if (args[0] === "network") return { code: 0, stdout: "chariox-slice-existing-net\n", stderr: "" }
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
    assert.deepEqual(result.docker.images, ["chariox/browser:fixture"])
    assert.deepEqual(result.docker.networks, ["chariox-slice-existing-net"])
    assert.deepEqual(commands.map((command) => command.slice(0, 2)), [
      ["docker", "ps"],
      ["docker", "volume"],
      ["docker", "image"],
      ["docker", "network"],
      ["vm_stat"],
    ])

    assert.equal(evaluateBrowserComputerDockerPreconditions({
      action: "save",
      before: result,
      imageRef: "chariox/browser:fixture",
      savePath: "/tmp/browser-state.tar",
      command: ["docker", "save", "chariox/browser:fixture", "-o", "/tmp/browser-state.tar"],
    }).ok, true)
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

test("documented resource ceilings derive a complete stricter guard cap", () => {
  const ceilings = {
    maximumRssBytes: 2_000_000_000,
    maximumPostRunDiskDeltaBytes: 10_000_000,
  }
  assert.deepEqual(deriveBrowserComputerCapsFromResourceCeilings(ceilings), {
    diskBytes: 10_000_000,
    memoryBytes: 2_000_000_000,
    processCount: 30,
    logBytes: 10_000_000,
  })
  assert.deepEqual(resolveBrowserComputerCaps({
    caps: { diskBytes: 9, memoryBytes: 19, processCount: 4, logBytes: 5 },
    resourceCeilings: ceilings,
  }), { diskBytes: 9, memoryBytes: 19, processCount: 4, logBytes: 5 })
  assert.throws(() => resolveBrowserComputerCaps({
    caps: { diskBytes: 10_000_001, memoryBytes: 19, processCount: 4, logBytes: 5 },
    resourceCeilings: ceilings,
  }), /exceeds its runbook ceiling/)
})

test("resource telemetry requires an authoritative managed target or explicit local fallback", () => {
  const managed = { telemetry: { scope: "managed-target", authoritative: true, source: "agent-1", targetId: "machine-1" } }
  assert.equal(assertBrowserComputerResourceTelemetry(managed).mode, "managed-target")
  const mismatched = evaluateBrowserComputerResourceTelemetry(managed, { expectedTargetIds: ["machine-2"] })
  assert.equal(mismatched.ok, false)
  assert.match(mismatched.violations.join("\n"), /does not match an expected machine/)
  assert.equal(evaluateBrowserComputerResourceTelemetry({}).ok, false)
  assert.throws(() => assertBrowserComputerResourceTelemetry({}), /remote\/local host fallback is forbidden/)
  assert.equal(evaluateBrowserComputerResourceTelemetry({
    telemetry: { scope: "local-host", authoritative: true, fallback: true },
  }, { allowLocalFallback: true }).mode, "local-fallback")
})

test("watchdog rejects a transient owned-workload resource spike", () => {
  const result = evaluateBrowserComputerResourceWatchdogSample(
    resourceSamples()[0],
    { phase: "watchdog", ...snapshot({ diskAvailableBytes: 70, memoryAvailableBytes: 60, processCount: 6, logBytes: 10 }) },
    { diskBytes: 10, memoryBytes: 20, processCount: 4, logBytes: 5 },
  )
  assert.equal(result.ok, false)
  assert.match(result.violations.join("\n"), /watchdog disk growth 30 bytes/)
  assert.match(result.violations.join("\n"), /watchdog peak process count 6/)
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
      networks: [],
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
    networks: [],
  })
  assert.equal(result.memoryAvailableDeltaBytes, 5)
  assert.equal(result.diskAvailableDeltaBytes, -2)
})

test("cleanup rejects disappearance of pre-existing unowned resources", async () => {
  const result = await evaluateBrowserComputerCleanup({
    before: snapshot({
      containers: ["chariox-slice-unrelated"],
      volumes: ["chariox-slice-unrelated-home"],
    }),
    after: snapshot(),
  })

  assert.equal(result.ok, false)
  assert.match(result.violations.join("\n"), /pre-existing unowned container disappeared: chariox-slice-unrelated/)
  assert.match(result.violations.join("\n"), /pre-existing unowned volume disappeared: chariox-slice-unrelated-home/)
  assert.deepEqual(result.cleanupAccounting.removedUnowned, {
    containers: ["chariox-slice-unrelated"],
    volumes: ["chariox-slice-unrelated-home"],
    networks: [],
  })
})

test("Docker save/remove/restore preconditions fail closed and reject broad prune", () => {
  const inventory = {
    memory: { totalBytes: 100, availableBytes: 80 },
    disk: { totalBytes: 100, availableBytes: 80 },
    docker: {
      containers: ["chariox-slice-owned"],
      volumes: ["chariox-slice-owned-home"],
      networks: ["chariox-slice-owned-net"],
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
  const wrongSavePath = evaluateBrowserComputerDockerPreconditions({
    action: "save",
    before: inventory,
    imageRef: "chariox/browser:fixture",
    savePath: "/tmp/browser-state.tar",
    command: ["docker", "save", "chariox/browser:fixture", "-o", "/tmp/other-state.tar"],
  })
  assert.equal(wrongSavePath.ok, false)
  assert.match(wrongSavePath.violations.join("\n"), /archive path does not equal the declared savePath/)

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
  const parsedExtraRemove = parseBrowserComputerDockerMutationArgv(
    ["docker", "rm", "chariox-slice-owned", "unrelated"],
    "remove",
  )
  assert.deepEqual(parsedExtraRemove.affected.containers, ["chariox-slice-owned", "unrelated"])
  const extraRemove = evaluateBrowserComputerDockerPreconditions({
    action: "remove",
    before: inventory,
    ownedContainers: ["chariox-slice-owned"],
    targetContainers: ["chariox-slice-owned"],
    saved: true,
    command: ["docker", "rm", "chariox-slice-owned", "unrelated"],
  })
  assert.equal(extraRemove.ok, false)
  assert.match(extraRemove.violations.join("\n"), /affected resources do not equal declared targets/)
  assert.equal(evaluateBrowserComputerDockerPreconditions({
    action: "remove",
    before: inventory,
    ownedContainers: ["chariox-slice-owned"],
    targetContainers: ["chariox-slice-owned"],
    saved: true,
    command: ["docker", "rm", "chariox-slice-owned"],
  }).ok, true)
  const restoreCommand = [
    "docker", "create", "--name", "chariox-slice-owned",
    "--volume", "chariox-slice-owned-home:/data",
    "--network", "chariox-slice-owned-net", "chariox/browser:fixture",
  ]
  const parsedRestore = parseBrowserComputerDockerMutationArgv(restoreCommand, "restore")
  assert.deepEqual(parsedRestore.affected, {
    containers: ["chariox-slice-owned"],
    volumes: ["chariox-slice-owned-home"],
    images: ["chariox/browser:fixture"],
    networks: ["chariox-slice-owned-net"],
    mounts: ["chariox-slice-owned-home:/data"],
    mountSources: ["chariox-slice-owned-home"],
  })
  assert.equal(evaluateBrowserComputerDockerPreconditions({
    action: "restore",
    before: inventory,
    ownedContainers: ["chariox-slice-owned"],
    ownedVolumes: ["chariox-slice-owned-home"],
    ownedNetworks: ["chariox-slice-owned-net"],
    targetContainers: ["chariox-slice-owned"],
    targetVolumes: ["chariox-slice-owned-home"],
    targetNetworks: ["chariox-slice-owned-net"],
    targetMounts: ["chariox-slice-owned-home:/data"],
    imageRef: "chariox/browser:fixture",
    saved: true,
    removed: true,
    command: restoreCommand,
  }).ok, true)
  const mountRestoreCommand = [
    "docker", "create", "--name", "chariox-slice-owned",
    "--mount", "type=volume,source=chariox-slice-owned-home,target=/data",
    "--network", "chariox-slice-owned-net", "chariox/browser:fixture",
  ]
  const parsedMountRestore = parseBrowserComputerDockerMutationArgv(mountRestoreCommand, "restore")
  assert.deepEqual(parsedMountRestore.affected, {
    containers: ["chariox-slice-owned"],
    volumes: ["chariox-slice-owned-home"],
    images: ["chariox/browser:fixture"],
    networks: ["chariox-slice-owned-net"],
    mounts: ["type=volume,source=chariox-slice-owned-home,target=/data"],
    mountSources: ["chariox-slice-owned-home"],
  })
  assert.equal(evaluateBrowserComputerDockerPreconditions({
    action: "restore",
    before: inventory,
    ownedContainers: ["chariox-slice-owned"],
    ownedVolumes: ["chariox-slice-owned-home"],
    ownedNetworks: ["chariox-slice-owned-net"],
    targetContainers: ["chariox-slice-owned"],
    targetVolumes: ["chariox-slice-owned-home"],
    targetNetworks: ["chariox-slice-owned-net"],
    targetMounts: ["type=volume,source=chariox-slice-owned-home,target=/data"],
    targetMountSources: ["chariox-slice-owned-home"],
    imageRef: "chariox/browser:fixture",
    saved: true,
    removed: true,
    command: mountRestoreCommand,
  }).ok, true)
  const malformedMount = parseBrowserComputerDockerMutationArgv([
    "docker", "create", "--mount", "type=volume,target=/data", "chariox/browser:fixture",
  ], "restore")
  assert.match(malformedMount.violations.join("\n"), /requires an exact source volume/)
  const wrongLoadPath = evaluateBrowserComputerDockerPreconditions({
    action: "restore",
    before: inventory,
    saved: true,
    removed: true,
    imageRef: "chariox/browser:fixture",
    restorePath: "/tmp/browser-state.tar",
    command: ["docker", "load", "-i", "/tmp/other-state.tar"],
  })
  assert.equal(wrongLoadPath.ok, false)
  assert.match(wrongLoadPath.violations.join("\n"), /archive path does not equal the declared restorePath/)
  const exactLoad = evaluateBrowserComputerDockerPreconditions({
    action: "restore",
    before: inventory,
    saved: true,
    removed: true,
    imageRef: "chariox/browser:fixture",
    restorePath: "/tmp/browser-state.tar",
    command: ["docker", "load", "-i", "/tmp/browser-state.tar"],
  })
  assert.equal(exactLoad.ok, true)
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

test("persistence evidence binds actual argv, requests, inventories, and seam checkpoints", () => {
  const evidence = persistenceEvidence()
  const plan = persistencePlan()
  const declarations = persistenceDeclarations()
  const planned = evaluateBrowserComputerPersistenceMutationSeams({
    result: plan,
    dockerPreconditions: declarations,
    mode: "plan",
  })
  assert.equal(planned.ok, true, planned.violations.join("\n"))
  assert.equal(planned.mutations.every((mutation) => mutation.receipt === null), true)

  const result = evaluateBrowserComputerPersistenceMutationSeams({
    result: evidence,
    plan,
    dockerPreconditions: declarations,
  })
  assert.equal(result.ok, true)
  assert.deepEqual(result.mutations.map(({ action }) => action), ["save", "remove", "restore"])

  const altered = structuredClone(evidence)
  altered.persistenceMutations[2].request.argv[2] = "--all"
  const failed = evaluateBrowserComputerPersistenceMutationSeams({
    result: altered,
    plan,
    dockerPreconditions: declarations,
  })
  assert.equal(failed.ok, false)
  assert.match(failed.violations.join("\n"), /argv does not equal the exact mutation argv|mutation plan|broad prune/)

  const brokenReceiptChain = structuredClone(evidence)
  brokenReceiptChain.persistenceMutations[1].saveReceipt = {
    ...brokenReceiptChain.persistenceMutations[0].receipt,
    id: "foreign-save-receipt",
  }
  const broken = evaluateBrowserComputerPersistenceMutationSeams({
    result: brokenReceiptChain,
    plan,
    dockerPreconditions: declarations,
  })
  assert.equal(broken.ok, false)
  assert.match(broken.violations.join("\n"), /exact successful save receipt/)

  const predeclared = evaluateBrowserComputerPersistenceMutationSeams({
    result: evidence,
    dockerPreconditions: declarations,
    mode: "plan",
  })
  assert.equal(predeclared.ok, false)
  assert.match(predeclared.violations.join("\n"), /must not contain pre-execution receipt evidence/)
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
  assert.deepEqual(result.faultCheckpoint, { requested: null, reached: null, exercised: null })
})

test("fault guard fails when a requested checkpoint is missing or repeated out of order", async () => {
  const missing = await runBrowserComputerFaultGuard({
    faultAt: "during-browser",
    operation: async ({ checkpoint }) => {
      await checkpoint("before-browser-start")
    },
    cleanup: async () => ({ clean: true }),
  })
  assert.equal(missing.status, "failed")
  assert.deepEqual(missing.faultCheckpoint, { requested: "during-browser", reached: 0, exercised: false })
  assert.match(missing.failure.message, /not reached exactly once/)

  const repeated = await runBrowserComputerFaultGuard({
    operation: async ({ checkpoint }) => {
      await checkpoint("before-browser-start")
      await checkpoint("before-browser-start")
    },
    cleanup: async () => ({ clean: true }),
  })
  assert.equal(repeated.status, "failed")
  assert.match(repeated.failure.message, /exactly once/)
})

test("fault guard treats unsuccessful cleanup results as failures", async () => {
  for (const cleanup of [
    async () => ({ clean: false }),
    async () => ({ ok: false, violations: ["owned container remains"] }),
  ]) {
    const result = await runBrowserComputerFaultGuard({ cleanup })
    assert.equal(result.status, "failed")
    assert.match(result.failure.message, /cleanup reported/)
  }
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
  const restoreArgv = [
    "docker", "create", "--name", "chariox-slice-owned",
    "--volume", "chariox-slice-owned-home:/data",
    "--network", "chariox-slice-owned-net", "chariox/browser:fixture",
  ]
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
        receipt: { ok: true, id: "restore-receipt-1", parentReceiptId: removeReceipt.id },
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
  return [
    {
      action: "save",
      imageRef: "chariox/browser:fixture",
      savePath: "/tmp/browser-state.tar",
    },
    {
      action: "remove",
      ownedContainers: ["chariox-slice-owned"],
      targetContainers: ["chariox-slice-owned"],
      saved: true,
    },
    {
      action: "restore",
      ownedContainers: ["chariox-slice-owned"],
      ownedVolumes: ["chariox-slice-owned-home"],
      ownedNetworks: ["chariox-slice-owned-net"],
      targetContainers: ["chariox-slice-owned"],
      targetVolumes: ["chariox-slice-owned-home"],
      targetNetworks: ["chariox-slice-owned-net"],
      targetMounts: ["chariox-slice-owned-home:/data"],
      imageRef: "chariox/browser:fixture",
      saved: true,
      removed: true,
    },
  ]
}

function snapshot({
  memoryAvailableBytes = 100,
  diskAvailableBytes = 100,
  containers = [],
  volumes = [],
  networks = [],
  processCount = 2,
  logBytes = 0,
} = {}) {
  return {
    memory: { totalBytes: 100, availableBytes: memoryAvailableBytes },
    disk: { totalBytes: 100, availableBytes: diskAvailableBytes },
    process: { count: processCount },
    logs: { bytes: logBytes },
    docker: { containers, volumes, networks },
  }
}

function resourceSamples({ during = {}, after = {} } = {}) {
  return [
    { phase: "before", ...snapshot() },
    { phase: "during", ...snapshot(during) },
    { phase: "after", ...snapshot(after) },
  ]
}
