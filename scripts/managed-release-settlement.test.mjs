import assert from "node:assert/strict"
import { createHash, randomUUID } from "node:crypto"
import { lstat, mkdir, mkdtemp, open, readFile, rename, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { test } from "node:test"
import {
  acquireManagedReleaseBuilderLease,
  isTerminalBuildHistoryRecord,
  parseBuildHistoryList,
  reconcileManagedReleaseBuild,
  validateBuildMetadata,
} from "./managed-release-settlement.mjs"
import { dirname, join, resolve } from "node:path"

const builderName = "chariox-managed-release"
const nodeName = "chariox-managed-release0"
const otherBuilderName = "other-managed-release"
const otherNodeName = "other-managed-release0"
const bareId = "a".repeat(64)
const secondBareId = "b".repeat(64)
const baselineId = "c".repeat(64)
const fullRef = `${builderName}/${nodeName}/${bareId}`
const baselineRef = `${builderName}/${nodeName}/${baselineId}`
const deadOwnerPid = 2_147_483_647
const deadReaperPid = 2_147_483_646

function historyRow(ref, overrides = {}) {
  const createdAt = new Date(Date.now() - 30_000).toISOString()
  return {
    ref,
    name: "managed-release-artifacts",
    status: "completed",
    created_at: createdAt,
    total_steps: 12,
    completed_steps: 12,
    cached_steps: 0,
    completed_at: new Date(Date.now() - 1_000).toISOString(),
    ...overrides,
  }
}

function historyOutput(...rows) {
  return rows.map((row) => JSON.stringify(row)).join("\n") + (rows.length ? "\n" : "")
}

function makeBarrier(overrides = {}) {
  const invocationId = "00000000-0000-4000-8000-000000000001"
  const stateDirectory = "/managed-release-state"
  const runDirectory = `${stateDirectory}/run-${invocationId}`
  const outputPath = "/release-output"
  const startedAt = new Date(Date.now() - 90_000).toISOString()
  const builderFingerprint = {
    name: builderName,
    nodes: [{
      name: nodeName,
      endpoint: "unix:///var/run/docker.sock",
      containerId: "d".repeat(64),
      startedAt: "2026-09-27T10:00:00.000Z",
      pidsLimit: 1024,
      effectiveCpus: 2,
      memoryBytes: 8 * 1024 ** 3,
      memoryWithSwapBytes: 8 * 1024 ** 3,
    }],
  }
  return {
    schemaVersion: 1,
    builderName,
    invocationId,
    sourceCommit: "1".repeat(40),
    sourceTree: "2".repeat(40),
    sourceDigest: `sha256:${"3".repeat(64)}`,
    sourceDirectory: `${runDirectory}/source`,
    runDirectory,
    outputPath,
    pendingDirectory: `/.new-release-output-${invocationId}`,
    startedAt,
    historyBaseline: [baselineRef],
    builderFingerprint,
    buildRef: null,
    ...overrides,
  }
}

function inspectRecord(barrier, id, overrides = {}) {
  return {
    Ref: id,
    Context: barrier.sourceDirectory,
    Target: "managed-release-artifacts",
    StartedAt: barrier.startedAt,
    CompletedAt: new Date(Date.parse(barrier.startedAt) + 60_000).toISOString(),
    Status: "completed",
    ...overrides,
  }
}

function leasePaths(home) {
  const stateDirectory = join(home, ".local", "state", "chariox", "managed-kernel-release")
  const builderKey = createHash("sha256").update(builderName).digest("hex")
  return {
    stateDirectory,
    lockDirectory: join(stateDirectory, `builder-${builderKey}.lock`),
  }
}

async function seedAbandonedReaper(home, {
  reaperPid = deadReaperPid,
  reaperOwnerToken,
  bindOwner = true,
} = {}) {
  const { stateDirectory, lockDirectory } = leasePaths(home)
  const ownerToken = randomUUID()
  await mkdir(stateDirectory, { recursive: true, mode: 0o700 })
  await mkdir(lockDirectory, { mode: 0o700 })
  await writeFile(join(lockDirectory, "owner.json"), JSON.stringify({ pid: deadOwnerPid, token: ownerToken }), {
    mode: 0o600,
  })
  const reaper = {
    pid: reaperPid,
    token: randomUUID(),
  }
  if (bindOwner) {
    reaper.ownerPid = deadOwnerPid
    reaper.ownerToken = reaperOwnerToken ?? ownerToken
  }
  await writeFile(join(lockDirectory, "reaper.json"), JSON.stringify(reaper), { mode: 0o600 })
  return { lockDirectory, ownerToken }
}

async function reconcile({ barrier = makeBarrier(), refs, records, fingerprint, restartProcessCwd } = {}) {
  const inspected = []
  const inspectionCwds = []
  const result = await reconcileManagedReleaseBuild({
    barrier,
    currentBuilderFingerprint: fingerprint ?? barrier.builderFingerprint,
    historyList: async () => refs,
    historyInspect: async (id, inspectionCwd) => {
      inspected.push(id)
      inspectionCwds.push({ id, inspectionCwd, restartProcessCwd })
      return records.get(id) ?? {}
    },
  })
  return { result, inspected, inspectionCwds }
}

test("managed release history parses empty, single-line, and multiple newline JSON records", () => {
  assert.deepEqual(parseBuildHistoryList(""), [])
  assert.deepEqual(parseBuildHistoryList(historyOutput(historyRow(fullRef))), [fullRef])

  const secondRef = `${builderName}/${nodeName}/${secondBareId}`
  const { completed_at: _completedAt, ...withoutOptionalCompletion } = historyRow(secondRef, { status: "running" })
  assert.deepEqual(
    parseBuildHistoryList(historyOutput(historyRow(fullRef), withoutOptionalCompletion)),
    [fullRef, secondRef],
  )
})

test("managed release history rejects duplicate, malformed, and incomplete CLI rows", () => {
  assert.throws(() => parseBuildHistoryList(historyOutput(historyRow(fullRef), historyRow(fullRef))), /duplicate/i)
  assert.throws(() => parseBuildHistoryList('{"ref":\n'), /malformed/i)
  assert.throws(() => parseBuildHistoryList(JSON.stringify([{ ID: bareId }])), /malformed/i)
  assert.throws(() => parseBuildHistoryList(historyOutput(historyRow("invalid-ref"))), /ref|malformed|invalid/i)
  assert.throws(() => parseBuildHistoryList(historyOutput(historyRow(fullRef, { completed_at: "not-a-timestamp" }))), /completion|timestamp|malformed/i)

  const { cached_steps: _cachedSteps, ...incomplete } = historyRow(fullRef)
  assert.throws(() => parseBuildHistoryList(historyOutput(incomplete)), /malformed|invalid/i)
})

test("metadata and inspect keep full builder references separate from bare inspect IDs", async () => {
  const metadataRef = validateBuildMetadata(
    { "buildx.build.ref": fullRef },
    builderName,
    [{ name: nodeName }],
  )
  assert.equal(metadataRef, fullRef)
  assert.throws(() => validateBuildMetadata({ "buildx.build.ref": bareId }, builderName, [{ name: nodeName }]))

  const barrier = makeBarrier({ buildRef: metadataRef })
  const { result, inspected } = await reconcile({
    barrier,
    records: new Map([[bareId, inspectRecord(barrier, bareId)]]),
  })
  assert.deepEqual(inspected, [bareId])
  assert.deepEqual(result, { settled: true, buildRef: fullRef, status: "completed" })
})

test("a persisted full build reference does not settle on mismatched or nonterminal inspect evidence", async () => {
  const barrier = makeBarrier({ buildRef: fullRef })
  const rejectedRecords = [
    inspectRecord(barrier, bareId, { Context: `${barrier.sourceDirectory}-other` }),
    inspectRecord(barrier, bareId, { Status: "running" }),
  ]

  for (const record of rejectedRecords) {
    const { result, inspected } = await reconcile({
      barrier,
      records: new Map([[bareId, record]]),
    })
    assert.deepEqual(inspected, [bareId])
    assert.equal(result.settled, false)
  }

  const changedFingerprint = structuredClone(barrier.builderFingerprint)
  changedFingerprint.nodes[0].endpoint = "unix:///var/run/other-docker.sock"
  const drifted = await reconcile({
    barrier,
    records: new Map([[bareId, inspectRecord(barrier, bareId)]]),
    fingerprint: changedFingerprint,
  })
  assert.equal(drifted.result.settled, false)
  assert.deepEqual(drifted.inspected, [])
})

test("history references with another builder, another node, or invalid identity do not settle", async () => {
  const barrier = makeBarrier()
  const wrongBuilder = `${otherBuilderName}/${nodeName}/${bareId}`
  const wrongNode = `${builderName}/${otherNodeName}/${bareId}`
  const invalidRef = `${builderName}/${nodeName}/${bareId}/extra`

  for (const ref of [wrongBuilder, wrongNode, invalidRef]) {
    const { result } = await reconcile({
      barrier,
      refs: [ref],
      records: new Map([[bareId, inspectRecord(barrier, bareId)]]),
    })
    assert.equal(result.settled, false, ref)
  }

  for (const reference of [wrongBuilder, wrongNode, invalidRef]) {
    assert.throws(() => validateBuildMetadata(
      { "buildx.build.ref": reference },
      builderName,
      [{ name: nodeName }],
    ))
  }
})

test("only one new record with matching invocation identity can settle", async () => {
  const barrier = makeBarrier()
  const secondRef = `${builderName}/${nodeName}/${secondBareId}`
  const records = new Map([
    [bareId, inspectRecord(barrier, bareId)],
    [secondBareId, inspectRecord(barrier, secondBareId)],
  ])

  const ambiguous = await reconcile({ barrier, refs: [fullRef, secondRef], records })
  assert.equal(ambiguous.result.settled, false)

  const wrongIdentityRecords = [
    inspectRecord(barrier, bareId, { Ref: secondBareId }),
    inspectRecord(barrier, bareId, { Context: `${barrier.sourceDirectory}-other` }),
    inspectRecord(barrier, bareId, { Context: undefined }),
    inspectRecord(barrier, bareId, { Target: "default" }),
    inspectRecord(barrier, bareId, { Target: undefined }),
    inspectRecord(barrier, bareId, { StartedAt: new Date(Date.parse(barrier.startedAt) - 10 * 60_000).toISOString() }),
  ]
  for (const record of wrongIdentityRecords) {
    const { result, inspected } = await reconcile({ barrier, refs: [fullRef], records: new Map([[bareId, record]]) })
    assert.equal(result.settled, false)
    assert.deepEqual(inspected, [bareId])
  }
})

test("restart inspection uses the saved absolute context regardless of the process CWD", async () => {
  const barrier = makeBarrier({ buildRef: fullRef })
  const restartProcessCwd = "/tmp/restarted-from-elsewhere"
  assert.notEqual(restartProcessCwd, barrier.sourceDirectory)

  const absoluteContext = await reconcile({
    barrier,
    restartProcessCwd,
    records: new Map([[bareId, inspectRecord(barrier, bareId)]]),
  })
  assert.equal(absoluteContext.result.settled, true)
  assert.deepEqual(absoluteContext.inspectionCwds, [{
    id: bareId,
    inspectionCwd: barrier.sourceDirectory,
    restartProcessCwd,
  }])

  const relativeBarrier = makeBarrier()
  const relativeContext = await reconcile({
    barrier: relativeBarrier,
    refs: [fullRef],
    restartProcessCwd,
    records: new Map([[bareId, inspectRecord(relativeBarrier, bareId, { Context: "./source" })]]),
  })
  assert.equal(relativeContext.result.settled, false)
  assert.deepEqual(relativeContext.inspectionCwds, [{
    id: bareId,
    inspectionCwd: relativeBarrier.sourceDirectory,
    restartProcessCwd,
  }])
  assert.equal(resolve(dirname(relativeBarrier.sourceDirectory), "./source"), relativeBarrier.sourceDirectory)
  assert.notEqual(resolve(relativeBarrier.sourceDirectory, "./source"), relativeBarrier.sourceDirectory)
  assert.notEqual(resolve(restartProcessCwd, "./source"), relativeBarrier.sourceDirectory)
})

test("lease recovery reclaims a dead owner-bound reaper marker and rejects unrelated or active markers", async (context) => {
  const matchingHome = await mkdtemp(join(tmpdir(), "chariox-release-dead-reaper-matching-"))
  context.after(() => rm(matchingHome, { recursive: true, force: true }))
  const matching = await seedAbandonedReaper(matchingHome)
  const lease = await acquireManagedReleaseBuilderLease({ home: matchingHome, builderName })
  await lease.release()
  assert.equal(await readFile(join(matching.lockDirectory, "owner.json")).then(() => true, () => false), false)

  for (const marker of [
    { homePrefix: "mismatched-owner", reaperPid: deadReaperPid, mismatchOwnerToken: true, bindOwner: true },
    { homePrefix: "unbound-reaper", reaperPid: deadReaperPid, mismatchOwnerToken: false, bindOwner: false },
    { homePrefix: "active-reaper", reaperPid: process.pid, mismatchOwnerToken: false, bindOwner: true },
  ]) {
    const home = await mkdtemp(join(tmpdir(), `chariox-release-${marker.homePrefix}-`))
    context.after(() => rm(home, { recursive: true, force: true }))
    const seeded = await seedAbandonedReaper(home, {
      reaperPid: marker.reaperPid,
      reaperOwnerToken: marker.mismatchOwnerToken ? randomUUID() : undefined,
      bindOwner: marker.bindOwner,
    })
    await assert.rejects(acquireManagedReleaseBuilderLease({ home, builderName }))
    assert.ok(await readFile(join(seeded.lockDirectory, "owner.json")))
    assert.ok(await readFile(join(seeded.lockDirectory, "reaper.json")))
  }
})

test("barrier removal flushes its directory through the lease file-operation seam", async (context) => {
  const home = await mkdtemp(join(tmpdir(), "chariox-release-barrier-remove-sync-"))
  context.after(() => rm(home, { recursive: true, force: true }))
  const { stateDirectory } = leasePaths(home)
  let directorySyncOpens = 0
  const fileOps = {
    open: async (path, ...args) => {
      if (path === stateDirectory) directorySyncOpens += 1
      return open(path, ...args)
    },
    lstat,
    readFile,
    rename,
    rm,
  }
  const lease = await acquireManagedReleaseBuilderLease({ home, builderName, fileOps })
  const invocationId = randomUUID()
  const runDirectory = join(stateDirectory, `run-${invocationId}`)
  const outputPath = join(home, "release-output")
  const barrier = makeBarrier({
    invocationId,
    runDirectory,
    sourceDirectory: join(runDirectory, "source"),
    runDirectoryIdentity: { dev: "1", ino: "2" },
    outputPath,
    pendingDirectory: join(home, `.new-release-output-${invocationId}`),
    pendingDirectoryIdentity: { dev: "1", ino: "3" },
    buildStarted: false,
  })
  try {
    await lease.writeBarrier(barrier)
    await lease.removeBarrier()
    assert.equal(directorySyncOpens, 2)
    await assert.rejects(readFile(lease.barrierPath), { code: "ENOENT" })
  } finally {
    await lease.release()
  }
})

test("nonterminal or malformed completion fields never prove a build terminal", async () => {
  const barrier = makeBarrier()
  const malformedOrNonterminal = [
    { Status: "running" },
    { Status: "completed", CompletedAt: "not-a-timestamp" },
    { Status: "completed", CompletedAt: null },
    {
      Status: "completed",
      CompletedAt: new Date(Date.parse(barrier.startedAt) - 1_000).toISOString(),
    },
  ]

  for (const completion of malformedOrNonterminal) {
    const record = inspectRecord(barrier, bareId, completion)
    assert.equal(isTerminalBuildHistoryRecord(record), false)
    const { result } = await reconcile({ barrier, refs: [fullRef], records: new Map([[bareId, record]]) })
    assert.equal(result.settled, false)
  }
})

test("terminal failure and cancellation statuses are terminal when inspection timestamps are valid", async () => {
  const barrier = makeBarrier({ buildRef: fullRef })
  for (const status of ["failed", "error", "canceled", "cancelled"]) {
    const record = inspectRecord(barrier, bareId, { Status: status })
    assert.equal(isTerminalBuildHistoryRecord(record), true, status)
    const { result, inspected } = await reconcile({
      barrier,
      records: new Map([[bareId, record]]),
    })
    assert.deepEqual(inspected, [bareId], status)
    assert.deepEqual(result, { settled: true, buildRef: fullRef, status }, status)
  }
})

test("a prepared barrier proves the Buildx command was not dispatched only with exact scratch identities", async () => {
  const barrier = makeBarrier({
    buildStarted: false,
    runDirectoryIdentity: { dev: "1", ino: "2" },
    pendingDirectoryIdentity: { dev: "1", ino: "3" },
  })
  let remoteHistoryReads = 0
  const result = await reconcileManagedReleaseBuild({
    barrier,
    currentBuilderFingerprint: barrier.builderFingerprint,
    historyList: async () => { remoteHistoryReads += 1; return [] },
    historyInspect: async () => { remoteHistoryReads += 1; return {} },
  })
  assert.deepEqual(result, { settled: true, buildRef: null, status: "not_started" })
  assert.equal(remoteHistoryReads, 0)

  const changedBuilder = structuredClone(barrier.builderFingerprint)
  changedBuilder.nodes[0].containerId = "e".repeat(64)
  const mismatched = await reconcileManagedReleaseBuild({
    barrier,
    currentBuilderFingerprint: changedBuilder,
    historyList: async () => { remoteHistoryReads += 1; return [] },
    historyInspect: async () => { remoteHistoryReads += 1; return {} },
  })
  assert.deepEqual(mismatched, { settled: false, reason: "builder fingerprint changed" })
  assert.equal(remoteHistoryReads, 0)
  await assert.rejects(reconcileManagedReleaseBuild({
    barrier: { ...barrier, pendingDirectoryIdentity: undefined },
    currentBuilderFingerprint: barrier.builderFingerprint,
    historyList: async () => [],
    historyInspect: async () => ({}),
  }), /unresolved build barrier is malformed/)
})

test("an unchanged baseline or any individually changed builder fingerprint field blocks settlement", async () => {
  const barrier = makeBarrier()
  const noNewRecords = await reconcile({ barrier, refs: [baselineRef], records: new Map() })
  assert.equal(noNewRecords.result.settled, false)
  assert.deepEqual(noNewRecords.inspected, [])

  const mutations = [
    (fingerprint) => { fingerprint.nodes[0].containerId = "e".repeat(64) },
    (fingerprint) => { fingerprint.nodes[0].startedAt = "2026-09-27T10:01:00.000Z" },
    (fingerprint) => { fingerprint.nodes[0].endpoint = "unix:///var/run/other-docker.sock" },
    (fingerprint) => { fingerprint.nodes[0].pidsLimit = 512 },
    (fingerprint) => { fingerprint.nodes[0].effectiveCpus = 4 },
    (fingerprint) => { fingerprint.nodes[0].memoryBytes = 7 * 1024 ** 3 },
    (fingerprint) => { fingerprint.nodes[0].memoryWithSwapBytes = 16 * 1024 ** 3 },
  ]
  for (const mutate of mutations) {
    const changedFingerprint = structuredClone(barrier.builderFingerprint)
    mutate(changedFingerprint)
    const changedBuilder = await reconcile({
      barrier,
      refs: [fullRef],
      records: new Map([[bareId, inspectRecord(barrier, bareId)]]),
      fingerprint: changedFingerprint,
    })
    assert.equal(changedBuilder.result.settled, false)
    assert.deepEqual(changedBuilder.inspected, [])
  }
})
