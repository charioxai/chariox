#!/usr/bin/env node
import { mkdir, writeFile } from "node:fs/promises"
import path from "node:path"

import {
  DEFAULT_DETERMINISTIC_RUNTIME_CHAOS_SEED,
  createDeterministicRuntimeChaosReplay,
} from "./lib/drill-deterministic-runtime-model.mjs"

const repoRoot = path.resolve(import.meta.dirname, "..", "..", "..")

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const outputPath = path.resolve(options.outputPath ?? path.join(
    repoRoot,
    ".artifacts",
    "chaos-replays",
    `runtime-${safeSegment(options.seed)}.json`,
  ))
  const replay = await createDeterministicRuntimeChaosReplay({ seed: options.seed })
  await mkdir(path.dirname(outputPath), { recursive: true })
  await writeFile(outputPath, `${JSON.stringify(replay, null, 2)}\n`, "utf8")
  console.log(JSON.stringify({
    status: replay.invariants.status,
    seed: replay.seed,
    traceEvents: replay.summary.traceEvents,
    artifactPath: outputPath,
  }))
}

function parseArgs(argv) {
  const options = {
    seed: DEFAULT_DETERMINISTIC_RUNTIME_CHAOS_SEED,
    outputPath: null,
    help: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--seed") options.seed = readValue(argv, index++, arg)
    else if (arg.startsWith("--seed=")) options.seed = arg.slice("--seed=".length)
    else if (arg === "--output") options.outputPath = readValue(argv, index++, arg)
    else if (arg.startsWith("--output=")) options.outputPath = arg.slice("--output=".length)
    else if (arg === "--help" || arg === "-h") options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function readValue(argv, index, flag) {
  const value = argv[index + 1]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
  return value
}

function safeSegment(value) {
  return String(value).replace(/[^a-zA-Z0-9._-]+/g, "-").replace(/^-+|-+$/g, "") || "seed"
}

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/deterministic-runtime-chaos-drill.mjs [options]",
    "",
    "Runs the seeded virtual-clock runtime convergence model and writes a replayable fault trace.",
    "",
    `  --seed VALUE   Replay seed (default ${DEFAULT_DETERMINISTIC_RUNTIME_CHAOS_SEED})`,
    "  --output PATH  Replay artifact path",
  ].join("\n"))
}

main().catch((error) => {
  console.error(`[deterministic-runtime-chaos] ${error.stack ?? error.message}`)
  process.exitCode = 1
})
