#!/usr/bin/env node

import path from "node:path"
import { fileURLToPath } from "node:url"

import { bounded, runLocalRustFaultDrill } from "./lib/local-rust-fault-drill-runtime.mjs"
import {
  SAVED_STATE_CORRUPTION_CASE_IDS,
  buildSavedStateCorruptionCargoArgs,
  parseSavedStateCorruptionProbe,
} from "./lib/saved-state-corruption-fault-drill.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")

runLocalRustFaultDrill({
  argv: process.argv.slice(2),
  repoRoot,
  name: "live-saved-state-corruption-fault-drill.mjs",
  description: "Runs the exact kernel probe for corrupt saved-state quarantine and known-good recovery.",
  schema: "chariox.saved_state_corruption_fault_drill.v1",
  caseIds: SAVED_STATE_CORRUPTION_CASE_IDS,
  cargoArgs: buildSavedStateCorruptionCargoArgs(),
  parseProbe: parseSavedStateCorruptionProbe,
  evidenceSubdir: "saved-state-corruption",
}).catch((error) => {
  console.error(`[saved-state-corruption-fault-drill] ${bounded(error.stack ?? error.message)}`)
  process.exitCode = 1
})
