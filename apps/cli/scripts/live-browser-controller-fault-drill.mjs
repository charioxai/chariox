#!/usr/bin/env node

import path from "node:path"
import { fileURLToPath } from "node:url"

import {
  BROWSER_CONTROLLER_FAULT_CASE_IDS,
  buildBrowserControllerFaultCargoArgs,
  parseBrowserControllerFaultProbe,
} from "./lib/browser-controller-fault-drill.mjs"
import { bounded, runLocalRustFaultDrill } from "./lib/local-rust-fault-drill-runtime.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const processNeedle = path.join(
  repoRoot,
  "apps", "kernel", "src", "runtime", "router", "tests", "room_environment_placement", "live_worker", "controller.fixture.mjs",
)

runLocalRustFaultDrill({
  argv: process.argv.slice(2),
  repoRoot,
  name: "live-browser-controller-fault-drill.mjs",
  description: "Runs the exact kernel library scenario that SIGKILLs a Room Browser Controller and proves recovery.",
  schema: "chariox.browser_controller_fault_drill.v1",
  caseIds: BROWSER_CONTROLLER_FAULT_CASE_IDS,
  cargoArgs: buildBrowserControllerFaultCargoArgs(),
  parseProbe: parseBrowserControllerFaultProbe,
  evidenceSubdir: "controller-faults",
  processNeedle,
}).catch((error) => {
  console.error(`[browser-controller-fault-drill] ${bounded(error.stack ?? error.message)}`)
  process.exitCode = 1
})
