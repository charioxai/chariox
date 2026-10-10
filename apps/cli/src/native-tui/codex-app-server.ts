import { type ChildProcess, spawn } from "node:child_process"
import net from "node:net"
import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"

export type CodexAppServerProcess = ChildProcess

export async function startCodexAppServer(endpoint: string, workingDirectory: string): Promise<CodexAppServerProcess> {
  const executable = process.env.CHARIOX_CODEX_BIN?.trim() || "codex"
  const child = spawn(executable, ["app-server", "--listen", endpoint], {
    cwd: workingDirectory,
    stdio: ["ignore", "ignore", "inherit"],
    env: process.env,
  })
  child.once("error", (error) => {
    throw error
  })
  const deadline = Date.now() + 15_000
  while (Date.now() < deadline) {
    if (await tcpEndpointIsReady(endpoint)) return child
    if (child.exitCode != null) throw new Error(`codex app-server exited before becoming ready: ${child.exitCode}`)
    await sleep(150)
  }
  throw new Error(`timed out waiting for codex app-server at ${endpoint}`)
}

async function tcpEndpointIsReady(endpoint: string): Promise<boolean> {
  const url = new URL(endpoint)
  return await new Promise((resolve) => {
    const socket = net.createConnection({
      host: url.hostname,
      port: Number(url.port),
    })
    const timer = setTimeout(() => {
      socket.destroy()
      resolve(false)
    }, 500)
    socket.once("connect", () => {
      clearTimeout(timer)
      socket.destroy()
      resolve(true)
    })
    socket.once("error", () => {
      clearTimeout(timer)
      resolve(false)
    })
  })
}
