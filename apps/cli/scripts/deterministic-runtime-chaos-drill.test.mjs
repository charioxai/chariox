import assert from "node:assert/strict"
import { execFile as execFileCallback } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { validateDrillChaosReplayBundle } from "./lib/drill-chaos-contract.mjs"
import { createDeterministicRuntimeChaosReplay } from "./lib/drill-deterministic-runtime-model.mjs"

const execFile = promisify(execFileCallback)
const scriptPath = fileURLToPath(new URL("./deterministic-runtime-chaos-drill.mjs", import.meta.url))

test("deterministic runtime chaos replay is stable and covers every fault primitive", async () => {
  const first = await createDeterministicRuntimeChaosReplay({ seed: "stable-replay" })
  const second = await createDeterministicRuntimeChaosReplay({ seed: "stable-replay" })

  assert.deepEqual(second, first)
  assert.equal(first.invariants.status, "passed")
  assert.deepEqual([...new Set(first.faultPlan.map((fault) => fault.kind))].sort(), [
    "delay",
    "drop",
    "duplicate",
    "process-death",
    "reorder",
    "route-partition",
    "route-reconnect",
    "stale-callback",
  ])
  assert(first.trace.some((event) => event.kind === "client.snapshot-applied"))
  assert(first.trace.some((event) => event.kind === "kernel.duplicate-operation-ignored"))
  assert.equal(first.summary.staleCallbacksSuppressed, 1)
})

test("deterministic runtime chaos CLI writes a validated replay artifact", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-deterministic-chaos-"))
  const outputPath = path.join(root, "replay.json")
  try {
    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--seed",
      "cli-replay",
      "--output",
      outputPath,
    ])
    const replay = JSON.parse(await readFile(outputPath, "utf8"))
    validateDrillChaosReplayBundle(replay)
    assert.match(stdout, /"status":"passed"/)
    assert.match(stdout, /"artifactPath":/)
    assert.equal(replay.seed, "cli-replay")
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
