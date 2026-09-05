#!/usr/bin/env node

import path from "node:path"
import { fileURLToPath } from "node:url"

import { bounded, runLocalRustFaultDrill } from "./lib/local-rust-fault-drill-runtime.mjs"
import {
  SLICE_SAVE_ACK_LOSS_CASE_IDS,
  buildSliceSaveAckLossCargoArgs,
  parseSliceSaveAckLossProbe,
} from "./lib/slice-save-ack-loss-fault-drill.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")

runLocalRustFaultDrill({
  argv: process.argv.slice(2),
  repoRoot,
  name: "live-slice-save-ack-loss-fault-drill.mjs",
  description: "Runs the exact kernel probe for idempotent slice-save acknowledgement replay.",
  schema: "chariox.slice_save_ack_loss_fault_drill.v1",
  caseIds: SLICE_SAVE_ACK_LOSS_CASE_IDS,
  cargoArgs: buildSliceSaveAckLossCargoArgs(),
  parseProbe: parseSliceSaveAckLossProbe,
  evidenceSubdir: "slice-save-ack-loss",
}).catch((error) => {
  console.error(`[slice-save-ack-loss-fault-drill] ${bounded(error.stack ?? error.message)}`)
  process.exitCode = 1
})
