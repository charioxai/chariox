#!/usr/bin/env node

import path from "node:path"
import { fileURLToPath } from "node:url"

import { bounded, runLocalRustFaultDrill } from "./lib/local-rust-fault-drill-runtime.mjs"
import {
  MEMORY_PRESSURE_ADMISSION_CASE_IDS,
  buildMemoryPressureAdmissionCargoArgs,
  parseMemoryPressureAdmissionProbe,
} from "./lib/memory-pressure-admission-fault-drill.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")

runLocalRustFaultDrill({
  argv: process.argv.slice(2),
  repoRoot,
  name: "live-memory-pressure-admission-fault-drill.mjs",
  description: "Runs the exact kernel probe for bounded local slice memory admission and recovery.",
  schema: "chariox.memory_pressure_admission_fault_drill.v1",
  caseIds: MEMORY_PRESSURE_ADMISSION_CASE_IDS,
  cargoArgs: buildMemoryPressureAdmissionCargoArgs(),
  parseProbe: parseMemoryPressureAdmissionProbe,
  evidenceSubdir: "memory-pressure-admission",
}).catch((error) => {
  console.error(`[memory-pressure-admission-fault-drill] ${bounded(error.stack ?? error.message)}`)
  process.exitCode = 1
})
