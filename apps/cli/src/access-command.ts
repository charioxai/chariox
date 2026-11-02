import { kernelUnixClient } from "./kernel-unix-client.js"
import { spawn } from "node:child_process"
import { defaultAccessHolder } from "./access-holder.js"
import { existsSync, mkdirSync, readFileSync, rmSync, unlinkSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { kernelSocketConfigRoot } from "@chariox/kernel-client/kernel-unix-socket-path"
import { LocalIpcClient, LocalIpcError } from "@chariox/kernel-client/ipc"
import type { KernelAccessCoveredResponse, KernelAccessGrantedResponse, KernelAccessGrantsListedResponse, KernelAccessRevokedResponse } from "@chariox/kernel-client/kernel-types"
import { CommandFailure, parseFlags, usageOf } from "./agent-command-table.js"
import { failureOf, probeKernel, reportFailure } from "./kernels-command.js"

const WAITER_START_TIMEOUT_MS = 10_000
// The kernel verifies the holder through the requester's live parent chain when the request
// arrives, so the parent must outlive the moment the waiter's request reaches the socket.
// The approval, minutes later, no longer needs the waiter inside the holder.
const WAITER_REQUEST_GRACE_MS = 250
const WAITER_ENV = "CHARIOX_ACCESS_WAITER"

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

function waiterOutcomeFile(holder: number): string {
  return join(process.env.XDG_RUNTIME_DIR || kernelSocketConfigRoot(), `access-waiter-${holder}.outcome`)
}

/** Why the waiter ended without a grant, so `access status` can say it instead of "not_requested". */
function waiterOutcome(holder: number): { code: string; message: string } | null {
  try {
    const { code, message } = JSON.parse(readFileSync(waiterOutcomeFile(holder), "utf8")) as { code: string; message: string }
    return typeof code === "string" && typeof message === "string" ? { code, message } : null
  } catch {
    return null
  }
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
  rmSync(waiterOutcomeFile(holder), { force: true })
  const child = spawn(process.execPath, [...script, "access", "request", ...args, ...(args.includes("--holder-pid") ? [] : ["--holder-pid", String(holder)])], { detached: true, stdio: ["ignore", "ignore", "pipe"], env: { ...process.env, [WAITER_ENV]: "1" } })
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

function recordWaiterOutcome(holder: number, error: unknown): void {
  const code = error instanceof LocalIpcError ? error.code : "cli_error"
  const message = error instanceof Error ? error.message : String(error)
  mkdirSync(join(waiterOutcomeFile(holder), ".."), { recursive: true, mode: 0o700 })
  writeFileSync(waiterOutcomeFile(holder), JSON.stringify({ code, message }), { mode: 0o600 })
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
  } catch (error) {
    if (process.env[WAITER_ENV]) recordWaiterOutcome(holder, error)
    throw error
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
  let reason: { code: string; message: string } | null = null
  try {
    await probeKernel(flags.get("--socket") as string | undefined)
    status = "granted"
  } catch (error) {
    const code = error instanceof LocalIpcError ? error.code : null
    if (code === "kernel_access_ended") status = "ended"
    else if (code === "kernel_access_not_covered") {
      if (waiterAlive(holder)) status = "pending"
      else {
        reason = waiterOutcome(holder)
        status = reason ? "ended" : "not_requested"
      }
      // The refusal's own hint: a status of "not_requested" alone is a dead end for an uncovered process.
      if (status === "not_requested") process.stderr.write(`${failureOf(error).message}\n`)
    } else throw error
  }
  console.log(JSON.stringify(reason ? { status, reason: reason.code, message: reason.message } : { status }))
}

const ACCESS_USAGE = [usageOf("access request"), usageOf("access status"), usageOf("access cover"), "chariox access list", "chariox access revoke <id|--all>"].join(" | ")

export async function runAccessCommand(argv: string[]): Promise<boolean> {
  if (argv[0] !== "access") return false
  if (argv[1] === "--help") {
    console.log(ACCESS_USAGE)
    return true
  }
  const direct = new Map([
    ["request", requestAccess], ["status", accessStatus], ["cover", coverProcess],
  ]).get(argv[1] ?? "")
  try {
    if (direct) await direct(argv.slice(2))
    else await listOrRevoke(argv)
  } catch (error) { reportFailure(error) }
  return true
}

async function listOrRevoke(argv: string[]): Promise<void> {
  const endpoint = process.env.CHARIOX_KERNEL_URL || `ws://${process.env.CHARIOX_KERNEL_HOST || "127.0.0.1"}:${process.env.CHARIOX_KERNEL_PORT || "43118"}`
  if (!(argv[1] === "list" && argv.length === 2) && !(argv[1] === "revoke" && argv.length === 3)) throw new CommandFailure("usage", ACCESS_USAGE)
  const client = new LocalIpcClient(endpoint)
  try {
    if (argv[1] === "list") {
      const result = await client.send<KernelAccessGrantsListedResponse>({ ListKernelAccessGrants: {} })
      console.log(JSON.stringify(result.KernelAccessGrantsListed, null, 2))
    } else {
      const result = await client.send<KernelAccessRevokedResponse>({ RevokeKernelAccessGrant: { grant_id: argv[2] === "--all" ? null : argv[2] } })
      console.log(`Revoked ${result.KernelAccessRevoked.revoked} grants`)
    }
  } finally { await client.close() }
}
