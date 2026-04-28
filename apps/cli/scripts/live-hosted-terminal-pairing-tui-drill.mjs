#!/usr/bin/env node
import { spawn } from "node:child_process"
import path from "node:path"
import { fileURLToPath } from "node:url"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const drill = path.join(scriptDir, "live-hosted-cloud-relay-drill.mjs")

const child = spawn(process.execPath, [drill], {
  cwd: path.resolve(scriptDir, "..", "..", ".."),
  env: {
    ...process.env,
    ARROBA_CLOUD_HOSTED_REMOTE_CLI_PAIRING: "1",
    ARROBA_CLOUD_HOSTED_REMOTE_CLI_PROVIDER:
      process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_PROVIDER ?? "codex",
    ARROBA_CLOUD_HOSTED_REMOTE_CLI_MODEL:
      process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_MODEL ?? "gpt-5.2-codex",
    ARROBA_CLOUD_HOSTED_REMOTE_CLI_EFFORT:
      process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_EFFORT ?? "low",
  },
  stdio: "inherit",
})

child.on("error", (error) => {
  console.error(error)
  process.exitCode = 1
})

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal)
    return
  }
  process.exitCode = code ?? 1
})
