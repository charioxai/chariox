import { spawn } from "node:child_process"
import { mkdtemp, rm } from "node:fs/promises"
import { tmpdir } from "node:os"
import { join } from "node:path"
import { fileURLToPath } from "node:url"

import { assert, test } from "../index.test-support.js"
import { PUBLICATION_PARENT_PIPE_ENV } from "../publication-parent-pipe.js"

const gatewayEntry = fileURLToPath(new URL("../index.js", import.meta.url))

// Stands in for the kernel: launches the gateway with a stdin pipe it keeps
// open, reports the gateway's pid, then idles until it is killed.
const parentScript = `
const { spawn } = require("node:child_process")
const gateway = spawn(process.execPath, [process.env.GATEWAY_ENTRY], {
  stdio: ["pipe", "ignore", "ignore"],
  env: process.env,
})
process.stdout.write(String(gateway.pid) + "\\n")
setInterval(() => {}, 60_000)
`

function isAlive(pid: number) {
  try {
    process.kill(pid, 0)
    return true
  } catch {
    return false
  }
}

async function waitFor(condition: () => boolean, timeoutMs: number) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (condition()) return true
    await new Promise((resolve) => setTimeout(resolve, 50))
  }
  return condition()
}

async function gatewayAfterParentKill(exitOnParentClose: boolean) {
  const logDir = await mkdtemp(join(tmpdir(), "chariox-gateway-parent-pipe-"))
  const env: NodeJS.ProcessEnv = {
    ...process.env,
    GATEWAY_ENTRY: gatewayEntry,
    HOST: "127.0.0.1",
    PORT: "0",
    CHARIOX_LOG_DIR: logDir,
    CHARIOX_LOG_LEVEL: "off",
  }
  for (const key of Object.keys(env)) {
    if (key.startsWith("CHARIOX_PUBLICATION_") || key.startsWith("CHARIOX_KERNEL_")) delete env[key]
  }
  // A standalone publication whose kernel is unreachable, so the test never
  // talks to a real kernel.
  env.CHARIOX_PUBLICATION_SESSION_ID = "session-1"
  env.CHARIOX_PUBLICATION_WORKFLOW = "workflow-1"
  env.CHARIOX_PUBLICATION_ENDPOINT = "endpoint-1"
  env.CHARIOX_KERNEL_URL = "ws://127.0.0.1:1"
  if (exitOnParentClose) env[PUBLICATION_PARENT_PIPE_ENV] = "1"
  const parent = spawn(process.execPath, ["-e", parentScript], {
    stdio: ["ignore", "pipe", "inherit"],
    env,
  })
  const gatewayPid = await new Promise<number>((resolve, reject) => {
    let output = ""
    parent.stdout.on("data", (chunk) => {
      output += chunk
      if (output.includes("\n")) resolve(Number(output.trim()))
    })
    parent.once("exit", () => reject(new Error("parent exited before reporting the gateway")))
  })
  try {
    // The gateway is up and serving before its parent dies.
    await new Promise((resolve) => setTimeout(resolve, 750))
    assert.equal(isAlive(gatewayPid), true, "gateway should be running before the parent dies")
    parent.kill("SIGKILL")
    return await waitFor(() => !isAlive(gatewayPid), exitOnParentClose ? 5_000 : 1_500)
  } finally {
    parent.kill("SIGKILL")
    if (isAlive(gatewayPid)) process.kill(gatewayPid, "SIGKILL")
    await rm(logDir, { recursive: true, force: true })
  }
}

test("a kernel-launched gateway exits when its parent is killed", async () => {
  const exited = await gatewayAfterParentKill(true)
  assert.equal(exited, true, "gateway must exit within 5s of its parent's SIGKILL")
})

test("a gateway not launched by a kernel ignores its stdin closing", async () => {
  const exited = await gatewayAfterParentKill(false)
  assert.equal(exited, false)
})
