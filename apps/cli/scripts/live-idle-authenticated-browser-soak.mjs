#!/usr/bin/env node

import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

import { parseIdleAuthenticatedBrowserSoakArgs } from "./lib/idle-authenticated-browser-soak.mjs"
import { runIdleAuthenticatedBrowserSoak } from "./lib/idle-authenticated-browser-soak-runtime.mjs"

const scriptPath = fileURLToPath(import.meta.url)
const repoRoot = path.resolve(path.dirname(scriptPath), "..", "..", "..")

function usage() {
  console.log([
    "Usage: node apps/cli/scripts/live-idle-authenticated-browser-soak.mjs [options]",
    "",
    "Runs the 24-hour idle Chromium/Browser Controller gate with a loopback-only synthetic authenticated session.",
    "Run --preflight and --smoke on the selected disposable non-development slice before --detach.",
    "",
    "  --preflight                       Validate dependencies, no competing owned run, and capacity",
    "  --smoke                           Deterministic 15-second smoke",
    "  --detach                          Preflight and launch in the background",
    "  --duration-seconds N              Duration (default and maximum: 86400)",
    "  --health-interval-seconds N       Health checkpoint interval (default: 30)",
    "  --sample-interval-seconds N       Resource sample interval (default: 30)",
    "  --max-cpu-percent N               Owned-process aggregate CPU ceiling (default: 300)",
    "  --max-rss-mb N                    Owned-process RSS ceiling (default: 2048)",
    "  --max-processes N                 Owned process-tree ceiling (default: 32)",
    "  --min-free-disk-mb N              Evidence-filesystem free-space floor (default: 1024)",
    "  --evidence-root PATH              External evidence root",
    "  --debug-port N                    Private Chromium CDP port (automatic by default)",
    "  --help                            Show this help",
  ].join("\n"))
}

try {
  const options = parseIdleAuthenticatedBrowserSoakArgs(process.argv.slice(2), { repoRoot, homeDir: os.homedir() })
  if (options.help) usage()
  else await runIdleAuthenticatedBrowserSoak({ options, repoRoot, scriptPath })
} catch (error) {
  console.error(`[idle-authenticated-browser-soak] ${String(error?.message ?? error).slice(0, 2_000)}`)
  process.exitCode = 1
}
