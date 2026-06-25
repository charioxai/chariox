#!/usr/bin/env node
import { spawn } from "node:child_process"
import { mkdir, rm, writeFile } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"

import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"
import { writeFakePiRpcHarness } from "./lib/fake-pi-rpc-harness.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")

function parseArgs(argv) {
  const options = {
    timeoutMs: 180_000,
    pollMs: 500,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--timeout-ms") options.timeoutMs = Number(argv[++index] ?? options.timeoutMs)
    else if (arg === "--poll-ms") options.pollMs = Number(argv[++index] ?? options.pollMs)
    else if (arg === "--help" || arg === "-h") {
      console.log("Usage: node apps/cli/scripts/live-pi-remote-provider-drill.mjs [--timeout-ms 180000] [--poll-ms 500]")
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  return options
}

function log(step, details) {
  if (details === undefined) console.log(`[pi-remote-provider-drill] ${step}`)
  else console.log(`[pi-remote-provider-drill] ${step}`, JSON.stringify(details))
}

async function run(command, args, options = {}) {
  await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: options.stdio ?? "inherit",
    })
    let stdout = ""
    let stderr = ""
    if (child.stdout) child.stdout.on("data", (chunk) => { stdout += chunk.toString() })
    if (child.stderr) child.stderr.on("data", (chunk) => { stderr += chunk.toString() })
    child.once("error", reject)
    child.once("exit", (code, signal) => {
      if (code === 0) resolve()
      else reject(new Error(`${command} ${args.join(" ")} exited with ${signal ?? code}\n${stdout}\n${stderr}`))
    })
  })
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const runId = `${process.pid}-${Date.now()}`
  const rootDir = path.join(repoRoot, ".artifacts", "live-pi-remote-provider-drill", runId)
  const fakePi = path.join(rootDir, "fake-pi.mjs")
  const fakePiLog = path.join(rootDir, "fake-pi.ndjson")
  const fakePiAuth = path.join(rootDir, "fake-pi-auth.json")
  let passed = false
  let failure = null
  try {
    await prepareDrillArtifacts(rootDir)
    await mkdir(rootDir, { recursive: true })
    await writeFile(fakePiAuth, JSON.stringify({ "openai-codex": { type: "oauth", accountId: "pi-remote-provider-drill" } }), "utf8")
    await writeFakePiRpcHarness(fakePi)
    log("build-binaries")
    await run("cargo", ["build", "--manifest-path", path.join(repoRoot, "apps/kernel/Cargo.toml"), "--bin", "arroba-kernel"])
    await run("cargo", ["build", "--manifest-path", path.join(repoRoot, "apps/relay/Cargo.toml"), "--bin", "arroba-relay"])
    log("run-remote-relay-drill", { fakePi })
    await run("node", [
      path.join(cliRoot, "scripts/live-remote-multi-agent-relay-drill.mjs"),
      "--providers",
      "pi,pi",
      "--provider-model",
      "pi=pi/openai-codex/gpt-5.4",
      "--timeout-ms",
      String(options.timeoutMs),
      "--poll-ms",
      String(options.pollMs),
    ], {
      cwd: repoRoot,
      env: {
        ...process.env,
        ARROBA_PI_BIN: fakePi,
        PI_AUTH_FILE: fakePiAuth,
        ARROBA_FAKE_PI_LOG: fakePiLog,
        ARROBA_FAKE_PI_SESSION_ID: `fake-pi-remote-session-${runId}`,
      },
      stdio: "inherit",
    })
    log("passed", { fakePiLog })
    passed = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    await finalizeDrillArtifacts({
      rootDir,
      passed,
      preserveOnFailure: true,
      preserveOnSuccess: false,
      failure,
      log,
      metadata: {
        drill: "live-pi-remote-provider",
        fakePi,
        fakePiAuth,
        fakePiLog,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      },
    })
    if (passed) await rm(rootDir, { recursive: true, force: true }).catch(() => {})
  }
}

main().catch((error) => {
  console.error(error?.stack ?? error?.message ?? String(error))
  process.exit(1)
})
