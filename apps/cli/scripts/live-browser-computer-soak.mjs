#!/usr/bin/env node

import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

import { parseBrowserComputerSoakArgs } from "./lib/browser-computer-soak.mjs"
import { redactEvidence, runBrowserComputerSoak } from "./lib/browser-computer-soak-runtime.mjs"

const scriptPath = fileURLToPath(import.meta.url)
const repoRoot = path.resolve(path.dirname(scriptPath), "..", "..", "..")

function usage() {
  console.log([
    "Usage: node apps/cli/scripts/live-browser-computer-soak.mjs [options]",
    "",
    "Runs an active Chromium, Browser Controller, Computer-input, and display-stream soak.",
    "A fresh matching preflight and smoke are mandatory before the eight-hour run.",
    "",
    "Options:",
    "  --preflight                       Validate dependencies and capacity only",
    "  --smoke                           Deterministic 12-second active smoke",
    "  --detach                          Verify preflight/smoke receipts and launch in background",
    "  --duration-seconds N              Active duration (default: 28800, maximum: 86400)",
    "  --activity-interval-seconds N     Browser mutation interval (default: 10)",
    "  --sample-interval-seconds N       Resource sampling interval (default: 10)",
    "  --evidence-root PATH              External evidence root",
    "  --display-number N                Isolated X display number (automatic by default)",
    "  --debug-port N                    Chromium CDP port (automatic by default)",
    "  --viewer-port N                   Private display-stream port (automatic by default)",
    "  --viewer-backend NAME             selkies (current) or novnc (never final-gate eligible)",
    "  --image-ref IMAGE                 Engine-local image tag or digest with a RepoDigest",
    "  --image-signature-key PATH        Cosign public key for signature and SLSA attestation",
    "  --container-engine NAME           docker (default) or podman",
    "  --runtime-container-id ID         Current digest-bound runtime container ID",
    "  --max-cadence-gap-seconds N       Fail on monotonic activity gaps (default: 30)",
    "  --max-rss-mib N                   Owned RSS bound (default: 4096)",
    "  --max-cpu-percent N               Owned aggregate CPU bound (default: 800)",
    "  --max-processes N                 Owned process-count bound (default: 96)",
    "  --max-disk-growth-mib N           Evidence/runtime disk-growth bound (default: 1024)",
    "  --max-open-files N                Owned open-file bound (default: 8192)",
    "  --max-network-mib N               Accounted owned-traffic bound (default: 65536)",
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
  console.error(`[browser-computer-soak] ${String(redactEvidence(error?.message ?? error)).slice(0, 2_000)}`)
  process.exitCode = 1
}
