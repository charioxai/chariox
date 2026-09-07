#!/usr/bin/env node

import path from "node:path"
import { fileURLToPath } from "node:url"

import { bounded, runLocalRustFaultDrill } from "./lib/local-rust-fault-drill-runtime.mjs"
import {
  SLICE_RESTORE_INTERRUPTION_CASE_IDS,
  SLICE_RESTORE_INTERRUPTION_TEST_NAME,
  buildSliceRestoreInterruptionCargoArgs,
  parseSliceRestoreInterruptionProbe,
} from "./lib/slice-restore-interruption-fault-drill.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")

runLocalRustFaultDrill({
  argv: process.argv.slice(2),
  repoRoot,
  name: "live-slice-restore-interruption-fault-drill.mjs",
  description: "Runs the exact kernel process-loss probe for startup rollback of an interrupted slice restore.",
  schema: "chariox.slice_restore_interruption_fault_drill.v1",
  caseIds: SLICE_RESTORE_INTERRUPTION_CASE_IDS,
  cargoArgs: buildSliceRestoreInterruptionCargoArgs(),
  parseProbe: parseSliceRestoreInterruptionProbe,
  evidenceSubdir: "slice-restore-interruption",
  processNeedle: SLICE_RESTORE_INTERRUPTION_TEST_NAME,
}).catch((error) => {
  console.error(`[slice-restore-interruption-fault-drill] ${bounded(error.stack ?? error.message)}`)
  process.exitCode = 1
})
