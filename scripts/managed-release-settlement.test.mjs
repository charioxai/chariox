import assert from "node:assert/strict"
import { test } from "node:test"
import {
  isTerminalBuildHistoryRecord,
  parseBuildHistoryList,
  reconcileManagedReleaseBuild,
  validateBuildMetadata,
} from "./managed-release-settlement.mjs"

const builderName = "chariox-managed-release"
const nodeName = "chariox-managed-release0"
const otherBuilderName = "other-managed-release"
const otherNodeName = "other-managed-release0"
const bareId = "a".repeat(64)
const secondBareId = "b".repeat(64)
const baselineId = "c".repeat(64)
const fullRef = `${builderName}/${nodeName}/${bareId}`
const baselineRef = `${builderName}/${nodeName}/${baselineId}`

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
    nodes: [{ name: nodeName, containerId: "d".repeat(64) }],
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

async function reconcile({ barrier = makeBarrier(), refs, records, fingerprint } = {}) {
  const inspected = []
  const result = await reconcileManagedReleaseBuild({
    barrier,
    currentBuilderFingerprint: fingerprint ?? barrier.builderFingerprint,
    historyList: async () => refs,
    historyInspect: async (id) => {
      inspected.push(id)
      return records.get(id) ?? {}
    },
  })
  return { result, inspected }
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

test("only one new record matching the exact source, start, and target can settle", async () => {
  const barrier = makeBarrier()
  const secondRef = `${builderName}/${nodeName}/${secondBareId}`
  const records = new Map([
    [bareId, inspectRecord(barrier, bareId)],
    [secondBareId, inspectRecord(barrier, secondBareId)],
  ])

  const ambiguous = await reconcile({ barrier, refs: [fullRef, secondRef], records })
  assert.deepEqual(ambiguous.result, {
    settled: false,
    reason: "history does not identify exactly one invocation",
  })

  const wrongIdentityRecords = [
    [inspectRecord(barrier, bareId, { Ref: secondBareId }), "Buildx history inspect reference does not match the listed build"],
    [inspectRecord(barrier, bareId, { Context: `${barrier.sourceDirectory}-other` }), "Buildx history inspect is missing or mismatches the invocation source context binding"],
    [inspectRecord(barrier, bareId, { Context: undefined }), "Buildx history inspect is missing or mismatches the invocation source context binding"],
    [inspectRecord(barrier, bareId, { Target: "default" }), "Buildx history inspect is missing or mismatches the managed release target binding"],
    [inspectRecord(barrier, bareId, { Target: undefined }), "Buildx history inspect is missing or mismatches the managed release target binding"],
    [inspectRecord(barrier, bareId, { StartedAt: new Date(Date.parse(barrier.startedAt) - 10 * 60_000).toISOString() }), "Buildx history inspect start timestamp does not match the invocation"],
  ]
  for (const [record, reason] of wrongIdentityRecords) {
    const { result } = await reconcile({ barrier, refs: [fullRef], records: new Map([[bareId, record]]) })
    assert.deepEqual(result, { settled: false, reason })
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

test("an unchanged baseline or changed builder fingerprint cannot create terminal proof", async () => {
  const barrier = makeBarrier()
  const noNewRecords = await reconcile({ barrier, refs: [baselineRef], records: new Map() })
  assert.equal(noNewRecords.result.settled, false)
  assert.deepEqual(noNewRecords.inspected, [])

  const changedFingerprint = structuredClone(barrier.builderFingerprint)
  changedFingerprint.nodes[0].containerId = "e".repeat(64)
  const changedBuilder = await reconcile({
    barrier,
    refs: [fullRef],
    records: new Map([[bareId, inspectRecord(barrier, bareId)]]),
    fingerprint: changedFingerprint,
  })
  assert.equal(changedBuilder.result.settled, false)
  assert.deepEqual(changedBuilder.inspected, [])
})
