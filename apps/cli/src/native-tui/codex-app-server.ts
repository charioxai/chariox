import { type ChildProcess, spawn } from "node:child_process"
import { closeSync, openSync, unlinkSync } from "node:fs"
import net from "node:net"
import path from "node:path"
import process from "node:process"
import { setTimeout as sleep } from "node:timers/promises"

import { LocalIpcClient } from "../ipc.js"
import {
  reserveLocalPort as reservePort,
  shellQuote,
} from "./launch-environment.js"

const reservedKernelPortLocks: string[] = []

export type CodexAppServerProcess = ChildProcess

export async function reserveCodexKernelServerPort(): Promise<number> {
  const range = process.env.CHARIOX_CODEX_KERNEL_SERVER_PORT_RANGE?.trim()
  if (!range) return reservePort()
  const match = range.match(/^(\d+)-(\d+)$/)
  if (!match) throw new Error("CHARIOX_CODEX_KERNEL_SERVER_PORT_RANGE must use START-END TCP port range syntax")
  const start = Number.parseInt(match[1]!, 10)
  const end = Number.parseInt(match[2]!, 10)
  if (!Number.isInteger(start) || !Number.isInteger(end) || start < 1 || end > 65535 || start > end) {
    throw new Error("CHARIOX_CODEX_KERNEL_SERVER_PORT_RANGE must be a valid TCP port range")
  }
  for (let port = start; port <= end; port += 1) {
    const lockPath = path.join("/tmp", `chariox-codex-kernel-server-port-${port}.lock`)
    try {
      const fd = openSync(lockPath, "wx")
      closeSync(fd)
      reservedKernelPortLocks.push(lockPath)
      return port
    } catch {
      continue
    }
  }
  throw new Error(`no available port in CHARIOX_CODEX_KERNEL_SERVER_PORT_RANGE=${range}`)
}

export function releaseKernelPortLocks() {
  while (reservedKernelPortLocks.length > 0) {
    const lockPath = reservedKernelPortLocks.pop()
    if (!lockPath) continue
    try {
      unlinkSync(lockPath)
    } catch {
      // best-effort cleanup for short-lived native TUI processes
    }
  }
}

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

export async function startCodexAppServerInKernel(options: {
  client: LocalIpcClient
  sessionId: string
  attachmentId: string
  endpoint: string
  listenEndpoint: string
  workingDirectory: string
}): Promise<string> {
  const executable = process.env.CHARIOX_CODEX_BIN?.trim() || "codex"
  const logFile = path.join("/tmp", `chariox-codex-kernel-server-${process.pid}-${Date.now()}.log`)
  const command = [
    `cd ${shellQuote(options.workingDirectory)}`,
    `(${shellQuote(executable)} app-server --listen ${shellQuote(options.listenEndpoint)} > ${shellQuote(logFile)} 2>&1 & echo $!)`,
  ].join(" && ")
  const response = await options.client.send<Record<string, unknown>>({
    RunShellCommand: {
      session_id: options.sessionId,
      attachment_id: options.attachmentId,
      command: "bash",
      args: ["-lc", command],
      working_directory: options.workingDirectory,
      timeout_ms: 5_000,
    },
  })
  const result = expectVariant<{ result: { exit_code: number; stdout: string; stderr: string } }>(
    response,
    "ShellCommandCompleted",
  ).result
  if (result.exit_code !== 0) {
    throw new Error(`failed to start codex app-server in kernel: ${result.stderr || result.stdout}`)
  }
  const pid = result.stdout.trim().split(/\s+/)[0]
  if (!pid) throw new Error("kernel codex app-server launch did not return a pid")
  const deadline = Date.now() + 15_000
  while (Date.now() < deadline) {
    if (await tcpEndpointIsReady(options.endpoint)) return pid
    await sleep(150)
  }
  throw new Error(`timed out waiting for kernel codex app-server at ${options.endpoint}; log ${logFile}`)
}

export async function stopCodexAppServerInKernel(
  client: LocalIpcClient,
  sessionId: string | null,
  attachmentId: string | null,
  pid: string,
  workingDirectory: string,
) {
  if (!sessionId || !attachmentId) return
  await client.send<Record<string, unknown>>({
    RunShellCommand: {
      session_id: sessionId,
      attachment_id: attachmentId,
      command: "bash",
      args: ["-lc", `kill ${shellQuote(pid)} 2>/dev/null || true`],
      working_directory: workingDirectory,
      timeout_ms: 5_000,
    },
  })
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

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`expected ${variant} response, received ${JSON.stringify(response)}`)
  }
  return response[variant] as T
}
