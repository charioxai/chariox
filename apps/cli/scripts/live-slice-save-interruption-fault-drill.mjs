#!/usr/bin/env node

import path from "node:path"
import { fileURLToPath } from "node:url"

import { bounded, runLocalRustFaultDrill } from "./lib/local-rust-fault-drill-runtime.mjs"
import {
  SLICE_SAVE_INTERRUPTION_CASE_IDS,
  buildSliceSaveInterruptionCargoArgs,
  parseSliceSaveInterruptionProbe,
} from "./lib/slice-save-interruption-fault-drill.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")

runLocalRustFaultDrill({
  argv: process.argv.slice(2),
  repoRoot,
  name: "live-slice-save-interruption-fault-drill.mjs",
  description: "Runs the exact kernel probe for saved-state publication interruption boundaries.",
  schema: "chariox.slice_save_interruption_fault_drill.v1",
  caseIds: SLICE_SAVE_INTERRUPTION_CASE_IDS,
  cargoArgs: buildSliceSaveInterruptionCargoArgs(),
  parseProbe: parseSliceSaveInterruptionProbe,
  evidenceSubdir: "slice-save-interruption",
}).catch((error) => {
  console.error(`[slice-save-interruption-fault-drill] ${bounded(error.stack ?? error.message)}`)
  process.exitCode = 1
})
