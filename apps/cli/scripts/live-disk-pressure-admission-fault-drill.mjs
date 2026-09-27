#!/usr/bin/env node

import path from "node:path"
import { fileURLToPath } from "node:url"

import { bounded, runLocalRustFaultDrill } from "./lib/local-rust-fault-drill-runtime.mjs"
import {
  DISK_PRESSURE_ADMISSION_CASE_IDS,
  buildDiskPressureAdmissionCargoArgs,
  parseDiskPressureAdmissionProbe,
} from "./lib/disk-pressure-admission-fault-drill.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")

runLocalRustFaultDrill({
  argv: process.argv.slice(2),
  repoRoot,
  name: "live-disk-pressure-admission-fault-drill.mjs",
  description: "Runs the exact kernel probe for fail-closed slice snapshot disk admission.",
  schema: "chariox.disk_pressure_admission_fault_drill.v1",
  caseIds: DISK_PRESSURE_ADMISSION_CASE_IDS,
  cargoArgs: buildDiskPressureAdmissionCargoArgs(),
  parseProbe: parseDiskPressureAdmissionProbe,
  evidenceSubdir: "disk-pressure-admission",
}).catch((error) => {
  console.error(`[disk-pressure-admission-fault-drill] ${bounded(error.stack ?? error.message)}`)
  process.exitCode = 1
})
