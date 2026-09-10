#!/usr/bin/env node

import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

import { parseBrowserComputerSoakArgs } from "./lib/browser-computer-soak.mjs"
import { runBrowserComputerSoak } from "./lib/browser-computer-soak-runtime.mjs"

const scriptPath = fileURLToPath(import.meta.url)
const repoRoot = path.resolve(path.dirname(scriptPath), "..", "..", "..")

function usage() {
  console.log([
    "Usage: node apps/cli/scripts/live-browser-computer-soak.mjs [options]",
    "",
    "Runs an active Chromium, Browser Controller, and Selkies display-stream soak.",
    "The normal duration is eight hours. Use --smoke before any full run.",
    "",
    "Options:",
    "  --preflight                       Validate dependencies and capacity only",
    "  --smoke                           Deterministic 12-second active smoke",
    "  --detach                          Preflight and launch in the background",
    "  --duration-seconds N              Active duration (default: 28800, maximum: 86400)",
    "  --activity-interval-seconds N     Browser mutation interval (default: 10)",
    "  --sample-interval-seconds N       Resource sampling interval (default: 10)",
    "  --evidence-root PATH              External evidence root",
    "  --display-number N                Isolated X display number (automatic by default)",
    "  --debug-port N                    Chromium CDP port (automatic by default)",
    "  --viewer-port N                   Private Selkies port (automatic by default)",
    "  --help                            Show this help",
  ].join("\n"))
}

try {
  const options = parseBrowserComputerSoakArgs(process.argv.slice(2), {
    repoRoot,
    homeDir: os.homedir(),
  })
  if (options.help) usage()
  else await runBrowserComputerSoak({ options, repoRoot, scriptPath })
} catch (error) {
  console.error(`[browser-computer-soak] ${String(error?.message ?? error).slice(0, 2_000)}`)
  process.exitCode = 1
}
