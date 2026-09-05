#!/usr/bin/env node

import path from "node:path"
import { fileURLToPath } from "node:url"

import { bounded, runLocalRustFaultDrill } from "./lib/local-rust-fault-drill-runtime.mjs"
import {
  QUEUE_SATURATION_FAULT_CASE_IDS,
  buildQueueSaturationFaultCargoArgs,
  parseQueueSaturationFaultProbe,
} from "./lib/queue-saturation-fault-drill.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")

runLocalRustFaultDrill({
  argv: process.argv.slice(2),
  repoRoot,
  name: "live-queue-saturation-fault-drill.mjs",
  description: "Runs the exact relay probe for full target queues and slow subscribers without closing healthy readers.",
  schema: "chariox.queue_saturation_fault_drill.v1",
  caseIds: QUEUE_SATURATION_FAULT_CASE_IDS,
  cargoArgs: buildQueueSaturationFaultCargoArgs(),
  parseProbe: parseQueueSaturationFaultProbe,
  evidenceSubdir: "queue-saturation-faults",
}).catch((error) => {
  console.error(`[queue-saturation-fault-drill] ${bounded(error.stack ?? error.message)}`)
  process.exitCode = 1
})
