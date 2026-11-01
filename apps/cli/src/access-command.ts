import { kernelUnixClient } from "./kernel-unix-client.js"
import { spawn } from "node:child_process"
import { defaultAccessHolder } from "./access-holder.js"
import { existsSync, mkdirSync, readFileSync, unlinkSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { kernelSocketConfigRoot } from "@chariox/kernel-client/kernel-unix-socket-path"
import { LocalIpcClient, LocalIpcError } from "@chariox/kernel-client/ipc"
import type { KernelAccessCoveredResponse, KernelAccessGrantedResponse, KernelAccessGrantsListedResponse, KernelAccessRevokedResponse } from "@chariox/kernel-client/kernel-types"
import { CommandFailure, parseFlags, usageOf } from "./agent-command-table.js"
import { probeKernel, reportFailure } from "./kernels-command.js"

const WAITER_START_TIMEOUT_MS = 10_000
// The kernel verifies the holder through the requester's live parent chain, so
// the parent must outlive the moment the waiter's request reaches the socket.
const WAITER_REQUEST_GRACE_MS = 250

function holderPid(flags: Map<string, string | true>, fallback: () => number): number {
  const holder = flags.has("--holder-pid") ? Number(flags.get("--holder-pid")) : fallback()
  if (!Number.isSafeInteger(holder) || holder <= 1) throw new CommandFailure("usage", "Access requires a positive integer holder pid")
  return holder
}

export function parseAccessRequest(argv: string[], fallbackHolder: () => number) {
  const flags = parseFlags("access request", argv)
  if (flags.has("--until-exit") && flags.has("--minutes")) throw new CommandFailure("usage", "Choose --until-exit or --minutes, not both")
  const minutes = flags.has("--minutes") ? Number(flags.get("--minutes")) : undefined
  if (minutes !== undefined && (!Number.isSafeInteger(minutes) || minutes < 1)) throw new CommandFailure("usage", "Access requires a positive integer lifetime")
  const holder_pid = holderPid(flags, fallbackHolder)
  const term = flags.has("--until-exit") ? "until_holder_exits" as const : minutes !== undefined ? { minutes } : undefined
  return {
    request: { RequestKernelAccess: { holder_pid, ...(term ? { term } : {}) } },
    socket: flags.get("--socket") as string | undefined,
    noWait: flags.has("--no-wait"),
  }
}

function waiterPidFile(holder: number): string {
  return join(process.env.XDG_RUNTIME_DIR || kernelSocketConfigRoot(), `access-waiter-${holder}.pid`)
}

function waiterAlive(holder: number): boolean {
  try {
    const pid = Number(readFileSync(waiterPidFile(holder), "utf8"))
    if (!Number.isSafeInteger(pid) || pid <= 1) throw new Error("Invalid access waiter pid")
    process.kill(pid, 0)
    return true
  } catch {
    try { unlinkSync(waiterPidFile(holder)) } catch { /* no waiter was recorded */ }
    return false
  }
}

function waiterFailure(output: string): CommandFailure {
  try {
    const { error } = JSON.parse(output) as { error: { code: string; message: string } }
    return new CommandFailure(error.code, error.message)
  } catch {
    return new CommandFailure("cli_error", `the access waiter failed: ${output.trim()}`)
  }
}

async function startWaiter(argv: string[], holder: number): Promise<void> {
  // Bun's embedded entry exists in its virtual filesystem, but the compiled
  // executable already owns that entry and accepts only CLI arguments.
  const entry = process.argv[1]
  const script = entry && !entry.startsWith("/$bunfs/") && existsSync(entry) ? [entry] : []
  const args = argv.filter(arg => arg !== "--no-wait")
  const child = spawn(process.execPath, [...script, "access", "request", ...args, ...(args.includes("--holder-pid") ? [] : ["--holder-pid", String(holder)])], { detached: true, stdio: ["ignore", "ignore", "pipe"] })
  const output = await new Promise<string>((resolve, reject) => {
    const timer = setTimeout(() => reject(new CommandFailure("cli_error", "the access waiter did not start")), WAITER_START_TIMEOUT_MS)
    child.stderr.once("data", chunk => { clearTimeout(timer); resolve(String(chunk)) })
    child.once("error", error => { clearTimeout(timer); reject(error) })
  }).catch(error => {
    if (typeof child.pid === "number" && Number.isSafeInteger(child.pid) && child.pid > 1) child.kill()
    throw error
  })
  if (!output.startsWith("Waiting")) throw waiterFailure(output)
  await new Promise(resolve => setTimeout(resolve, WAITER_REQUEST_GRACE_MS))
  mkdirSync(join(waiterPidFile(holder), ".."), { recursive: true, mode: 0o700 })
  writeFileSync(waiterPidFile(holder), String(child.pid), { mode: 0o600 })
  child.stderr.destroy()
  child.unref()
}

async function requestAccess(argv: string[]): Promise<void> {
  const { request, socket, noWait } = parseAccessRequest(argv, defaultAccessHolder)
  const holder = request.RequestKernelAccess.holder_pid
  if (noWait) {
    await startWaiter(argv, holder)
    console.log(JSON.stringify({ status: "pending", holder_pid: holder, note: "approve in your Chariox client, then run chariox access status" }))
    process.exitCode = 6
    return
  }
  const client = kernelUnixClient(socket)
  try {
    console.error(`Waiting for the kernel's access popup for holder pid ${holder}.`)
    const response = await client.send<KernelAccessGrantedResponse>(request)
    console.log(JSON.stringify(response.KernelAccessGranted.grant))
  } finally { await client.close() }
}

async function coverProcess(argv: string[]): Promise<void> {
  const flags = parseFlags("access cover", argv)
  const pid = Number(flags.get("--pid"))
  if (!Number.isSafeInteger(pid) || pid <= 1) throw new CommandFailure("usage", "Cover requires a positive integer --pid")
  const client = kernelUnixClient(flags.get("--socket") as string | undefined)
  try {
    const response = await client.send<KernelAccessCoveredResponse>({ CoverKernelAccessProcess: { pid } })
    console.log(JSON.stringify(response.KernelAccessCovered))
  } finally { await client.close() }
}

async function accessStatus(argv: string[]): Promise<void> {
  const flags = parseFlags("access status", argv)
  const holder = holderPid(flags, defaultAccessHolder)
  let status: "granted" | "pending" | "ended" | "not_requested"
  try {
    await probeKernel(flags.get("--socket") as string | undefined)
    status = "granted"
  } catch (error) {
    const code = error instanceof LocalIpcError ? error.code : null
    if (code === "kernel_access_ended") status = "ended"
    else if (code === "kernel_access_not_covered") status = waiterAlive(holder) ? "pending" : "not_requested"
    else throw error
  }
  console.log(JSON.stringify({ status }))
}

const ACCESS_USAGE = [usageOf("access request"), usageOf("access status"), usageOf("access cover"), "chariox access list", "chariox access revoke <id|--all>"].join(" | ")

export async function runAccessCommand(argv: string[]): Promise<boolean> {
  if (argv[0] !== "access") return false
  const direct = { request: requestAccess, status: accessStatus, cover: coverProcess }[argv[1] ?? ""]
  if (direct) {
    try {
      await direct(argv.slice(2))
    } catch (error) { reportFailure(error) }
    return true
  }
  const endpoint = process.env.CHARIOX_KERNEL_URL || `ws://${process.env.CHARIOX_KERNEL_HOST || "127.0.0.1"}:${process.env.CHARIOX_KERNEL_PORT || "43118"}`
  const client = new LocalIpcClient(endpoint)
  try {
    if (argv[1] === "list" && argv.length === 2) {
      const result = await client.send<KernelAccessGrantsListedResponse>({ ListKernelAccessGrants: {} })
      console.log(JSON.stringify(result.KernelAccessGrantsListed, null, 2))
    } else if (argv[1] === "revoke" && argv.length === 3) {
      const result = await client.send<KernelAccessRevokedResponse>({ RevokeKernelAccessGrant: { grant_id: argv[2] === "--all" ? null : argv[2] } })
      console.log(`Revoked ${result.KernelAccessRevoked.revoked} grants`)
    } else {
      throw new CommandFailure("usage", ACCESS_USAGE)
    }
  } finally { await client.close() }
  return true
}
