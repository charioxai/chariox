#!/usr/bin/env node
import { spawn } from "node:child_process"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const drill = path.join(scriptDir, "live-hosted-cloud-relay-drill.mjs")
const MAX_OUTPUT_CHARS = 128_000

function nowStamp() {
  return new Date().toISOString().replace(/[:.]/g, "-")
}

function appendOutput(buffer, chunk) {
  const next = buffer + chunk.toString("utf8")
  if (next.length <= MAX_OUTPUT_CHARS) return next
  return next.slice(next.length - MAX_OUTPUT_CHARS)
}

function tailLines(value, count = 120) {
  return value.split("\n").slice(-count).join("\n")
}

async function main() {
  const rootDir = path.join(repoRoot, ".artifacts", "live-hosted-terminal-pairing-tui-drill", nowStamp())
  await prepareDrillArtifacts(rootDir)
  let stdout = ""
  let stderr = ""
  let exitCode = null
  let exitSignal = null
  let failure = null
  const env = {
    ...process.env,
    CHARIOX_CLOUD_HOSTED_REMOTE_CLI_PAIRING: "1",
    CHARIOX_CLOUD_HOSTED_REMOTE_CLI_PROVIDER:
      process.env.CHARIOX_CLOUD_HOSTED_REMOTE_CLI_PROVIDER ?? "codex",
    CHARIOX_CLOUD_HOSTED_REMOTE_CLI_MODEL:
      process.env.CHARIOX_CLOUD_HOSTED_REMOTE_CLI_MODEL ?? "gpt-5.2-codex",
    CHARIOX_CLOUD_HOSTED_REMOTE_CLI_EFFORT:
      process.env.CHARIOX_CLOUD_HOSTED_REMOTE_CLI_EFFORT ?? "low",
  }

  try {
    await new Promise((resolve, reject) => {
      const child = spawn(process.execPath, [drill], {
        cwd: repoRoot,
        env,
        stdio: ["ignore", "pipe", "pipe"],
      })
      child.stdout.on("data", (chunk) => {
        stdout = appendOutput(stdout, chunk)
        process.stdout.write(chunk)
      })
      child.stderr.on("data", (chunk) => {
        stderr = appendOutput(stderr, chunk)
        process.stderr.write(chunk)
      })
      child.on("error", reject)
      child.on("exit", (code, signal) => {
        exitCode = code
        exitSignal = signal
        if (signal) {
          reject(new Error(`hosted terminal pairing drill exited with signal ${signal}`))
        } else if (code === 0) {
          resolve()
        } else {
          reject(new Error(`hosted terminal pairing drill exited with code ${code ?? "unknown"}`))
        }
      })
    })
    await finalizeDrillArtifacts({ rootDir, passed: true })
  } catch (error) {
    failure = error
    await finalizeDrillArtifacts({
      rootDir,
      passed: false,
      preserveOnFailure: true,
      failure,
      metadata: {
        drill: "live-hosted-terminal-pairing-tui",
        childDrill: drill,
        provider: env.CHARIOX_CLOUD_HOSTED_REMOTE_CLI_PROVIDER,
        model: env.CHARIOX_CLOUD_HOSTED_REMOTE_CLI_MODEL,
        effort: env.CHARIOX_CLOUD_HOSTED_REMOTE_CLI_EFFORT,
        exitCode,
        exitSignal,
        stdoutTail: tailLines(stdout),
        stderrTail: tailLines(stderr),
      },
    })
    throw error
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
