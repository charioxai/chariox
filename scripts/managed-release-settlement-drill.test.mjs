import assert from "node:assert/strict"
import { randomUUID } from "node:crypto"
import { mkdtemp, mkdir, readFile, realpath, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import test from "node:test"
import { acquireManagedReleaseBuilderLease } from "./managed-release-settlement.mjs"
import { OWNER_FILE, parseHistoryRows, removeOwnedDirectory, runSettlementScenario } from "./managed-release-settlement-drill.mjs"

const BUILDER = "chariox-settlement-test-00000000"
const NODE = "builder0"
const FINGERPRINT = {
  name: BUILDER,
  nodes: [{
    name: NODE,
    endpoint: "test-context",
    containerId: "a".repeat(64),
    startedAt: "2026-09-27T00:00:00.000Z",
    pidsLimit: 128,
    effectiveCpus: 1,
    memoryBytes: 1024 ** 3,
    memoryWithSwapBytes: 2 * 1024 ** 3,
    init: true,
    networkMode: "none",
  }],
}
const REPOSITORY = await realpath(dirname(dirname(fileURLToPath(import.meta.url))))

function historyRow(ref, status, completed = false) {
  return {
    ref,
    name: "settlement-fixture",
    status,
    created_at: "2026-09-27T00:00:00.000Z",
    total_steps: 3,
    completed_steps: status === "running" ? 1 : 3,
    cached_steps: 0,
    ...(completed ? { completed_at: "2026-09-27T00:01:00.000Z" } : {}),
  }
}

async function fixtureRoots() {
  const temporaryParent = await mkdtemp(join(tmpdir(), "managed-release-settlement-"))
  const parent = await realpath(temporaryParent)
  const scratchToken = randomUUID()
  const scratch = join(parent, `scratch-${scratchToken}`)
  const evidence = join(parent, "evidence-owned")
  const home = join(scratch, "home")
  await mkdir(scratch, { mode: 0o700 })
  await mkdir(evidence, { mode: 0o700 })
  await mkdir(home, { mode: 0o700 })
  await writeFile(join(scratch, OWNER_FILE), JSON.stringify({ kind: "scratch", token: scratchToken }), { mode: 0o600 })
  await writeFile(join(evidence, OWNER_FILE), JSON.stringify({ kind: "evidence", token: randomUUID() }), { mode: 0o600 })
  return {
    parent: temporaryParent,
    home,
    scratch: { directory: scratch, token: scratchToken, kind: "scratch" },
    evidence: { directory: evidence, token: "unused", kind: "evidence" },
  }
}

function fakeEngine(options = {}) {
  let row
  let terminalInspect
  let sourceDirectory
  let startTime
  let inspectCalls = 0
  const fingerprint = structuredClone(FINGERPRINT)
  return {
    async inspectBuilder() {
      inspectCalls += 1
      if (options.builderDrift && inspectCalls > 1) {
        return {
          ...fingerprint,
          nodes: [{ ...fingerprint.nodes[0], pidsLimit: 127 }],
        }
      }
      return structuredClone(fingerprint)
    },
    async historyList() {
      return row ? `${JSON.stringify(row)}\n` : ""
    },
    async historyInspect(id, cwd) {
      assert.equal(id, "solve-001", "production reconciliation must inspect with a bare Buildx ID")
      assert.equal(cwd, sourceDirectory, "inspect must run from the retained invocation source")
      return {
        Ref: options.foreignInspect ? "foreign-id" : id,
        Context: cwd,
        Target: "managed-release-artifacts",
        StartedAt: startTime,
        ...(terminalInspect ?? { Status: "running" }),
      }
    },
    async startBuild(context, { invocationId }) {
      sourceDirectory = context
      startTime = new Date().toISOString()
      row = historyRow(options.foreignHistory ? "another-builder/node0/foreign-id" : `${BUILDER}/${NODE}/solve-001`, "running")
      if (options.builderDrift) fingerprint.nodes[0].pidsLimit = 127
      let closed = false
      const completion = new Promise(() => {})
      return {
        completion,
        get closed() { return closed },
        async interrupt() {
          closed = true
          if (options.sourceDrift) await writeFile(join(sourceDirectory, "fixture.txt"), `mutated ${invocationId}\n`)
          row = historyRow(row.ref, "error", true)
          terminalInspect = { Status: "error", CompletedAt: new Date(Date.now() + 1_000).toISOString() }
          return { status: 130, signal: null, interrupted: true, reaped: true, streamsClosed: true }
        },
      }
    },
  }
}

async function runFake(options = {}) {
  const roots = await fixtureRoots()
  try {
    const result = await runSettlementScenario({
      ownedScratch: roots.scratch,
      evidence: roots.evidence,
      home: roots.home,
      builderName: BUILDER,
      baseImage: "busybox:stable@sha256:" + "b".repeat(64),
      repository: REPOSITORY,
      engine: fakeEngine(options),
      sleep: async () => {},
    })
    return { roots, result }
  } catch (error) {
    return { roots, error }
  }
}

test("parses actual newline-delimited Buildx history and its empty output", () => {
  assert.deepEqual(parseHistoryRows(""), [])
  const row = historyRow(`${BUILDER}/${NODE}/solve-001`, "running")
  assert.deepEqual(parseHistoryRows(`${JSON.stringify(row)}\n`), [{ ref: row.ref, status: "running" }])
  assert.throws(() => parseHistoryRows(`${JSON.stringify(historyRow(`${BUILDER}/${NODE}/solve-001`, "error"))}\n`), /missing a completion timestamp/)
})

test("persists a running full ref, rejects the successor, and settles from exact bare-ID terminal proof", async () => {
  const { roots, result, error } = await runFake()
  try {
    assert.ifError(error)
    assert.equal(result.outcome, "settled")
    assert.equal(result.terminalProof.fullRef, `${BUILDER}/${NODE}/solve-001`)
    assert.equal(result.terminalProof.status, "error")
    const report = JSON.parse(await readFile(result.evidencePath, "utf8"))
    assert.equal(report.scope, "BuildKit settlement seam only; no signed release acceptance")
    assert.deepEqual(report.events.map((event) => event.phase), [
      "builder-admitted",
      "barrier-persisted",
      "running-full-ref-persisted",
      "successor-rejected-unsettled",
      "client-interrupted",
      "terminal-proof",
    ])
    assert.equal(report.scratchRemoved, true)
  } finally {
    await rm(roots.parent, { recursive: true, force: true })
  }
})

test("preserves the barrier when new history belongs to a foreign builder", async () => {
  const { roots, error } = await runFake({ foreignHistory: true })
  try {
    assert.match(error.message, /foreign builder or node/)
    const lease = await acquireManagedReleaseBuilderLease({ home: roots.home, builderName: BUILDER })
    try {
      const barrier = await lease.readBarrier()
      assert.ok(barrier)
      assert.equal(barrier.buildRef, null)
    } finally {
      await lease.release()
    }
    assert.match(await readFile(join(roots.evidence.directory, "settlement-report.json"), "utf8"), /unproven/)
  } finally {
    await rm(roots.parent, { recursive: true, force: true })
  }
})

test("preserves the barrier when inspect returns a foreign bare reference", async () => {
  const { roots, error } = await runFake({ foreignInspect: true })
  try {
    assert.match(error.message, /reference does not match the listed build/)
    const lease = await acquireManagedReleaseBuilderLease({ home: roots.home, builderName: BUILDER })
    try {
      assert.equal((await lease.readBarrier()).buildRef, null)
    } finally {
      await lease.release()
    }
  } finally {
    await rm(roots.parent, { recursive: true, force: true })
  }
})

test("preserves the persisted barrier if the measured builder drifts", async () => {
  const { roots, error } = await runFake({ builderDrift: true })
  try {
    assert.match(error.message, /successor did not reject the unresolved invocation: builder fingerprint changed/)
    const lease = await acquireManagedReleaseBuilderLease({ home: roots.home, builderName: BUILDER })
    try {
      assert.equal((await lease.readBarrier()).buildRef, `${BUILDER}/${NODE}/solve-001`)
    } finally {
      await lease.release()
    }
  } finally {
    await rm(roots.parent, { recursive: true, force: true })
  }
})

test("preserves the persisted barrier if the retained source drifts", async () => {
  const { roots, error } = await runFake({ sourceDrift: true })
  try {
    assert.match(error.message, /retained source does not match its invocation digest/)
    const lease = await acquireManagedReleaseBuilderLease({ home: roots.home, builderName: BUILDER })
    try {
      assert.equal((await lease.readBarrier()).buildRef, `${BUILDER}/${NODE}/solve-001`)
    } finally {
      await lease.release()
    }
  } finally {
    await rm(roots.parent, { recursive: true, force: true })
  }
})

test("cleanup refuses to remove a scratch root with a foreign owner marker", async () => {
  const roots = await fixtureRoots()
  try {
    await writeFile(join(roots.scratch.directory, OWNER_FILE), JSON.stringify({ kind: "scratch", token: randomUUID() }))
    await assert.rejects(removeOwnedDirectory(roots.scratch), /foreign owner marker/)
    assert.equal((await readFile(join(roots.scratch.directory, OWNER_FILE), "utf8")) !== "", true)
  } finally {
    await rm(roots.parent, { recursive: true, force: true })
  }
})
