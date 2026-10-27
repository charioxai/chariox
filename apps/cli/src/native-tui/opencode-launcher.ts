import { type ChildProcess, spawn } from "node:child_process"
import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"

export type OpenCodeServerProcess = ChildProcess

export async function startOpenCodeServer(baseUrl: string, workingDirectory: string): Promise<OpenCodeServerProcess> {
  assertLocalStructuredEndpoint(baseUrl)
  const executable = process.env.CHARIOX_OPENCODE_BIN?.trim() || "opencode"
  const url = new URL(baseUrl)
  const child = spawn(executable, [
    "serve",
    "--hostname",
    url.hostname,
    "--port",
    url.port,
  ], {
    cwd: workingDirectory,
    stdio: ["ignore", "ignore", "inherit"],
    env: process.env,
  })
  child.once("error", (error) => {
    throw error
  })
  const deadline = Date.now() + 15_000
  while (Date.now() < deadline) {
    if (await openCodeHealthIsReady(baseUrl)) return child
    if (child.exitCode !== null) {
      throw new Error(`opencode serve exited before becoming ready with ${child.exitCode}`)
    }
    await sleep(100)
  }
  throw new Error(`timed out waiting for opencode serve at ${baseUrl}`)
}

export async function runOpenCodeAttach(options: {
  proxyUrl: string
  providerSessionId: string
  workingDirectory: string
}): Promise<void> {
  const executable = process.env.CHARIOX_OPENCODE_BIN?.trim() || "opencode"
  const args = [
    "attach",
    options.proxyUrl,
    "--session",
    options.providerSessionId,
    "--dir",
    options.workingDirectory,
  ]
  await new Promise<void>((resolve, reject) => {
    const child = spawn(executable, args, {
      stdio: "inherit",
      env: process.env,
    })
    child.once("error", reject)
    child.once("exit", (code, signal) => {
      if (code === 0) {
        resolve()
        return
      }
      reject(new Error(`opencode attach exited with ${signal ?? code}`))
    })
  })
}

function assertLocalStructuredEndpoint(endpoint: string) {
  const url = new URL(endpoint)
  if (url.hostname !== "127.0.0.1" && url.hostname !== "localhost") {
    throw new Error(`native OpenCode TUI mode only supports local provider endpoints for now; got ${endpoint}`)
  }
}

async function openCodeHealthIsReady(baseUrl: string): Promise<boolean> {
  try {
    const response = await fetch(new URL("/global/health", baseUrl))
    return response.ok
  } catch {
    return false
  }
}
