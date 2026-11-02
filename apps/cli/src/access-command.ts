import { kernelUnixClient } from "./kernel-unix-client.js"
import { LocalIpcClient } from "@chariox/kernel-client/ipc"
import type { KernelAccessCoveredResponse, KernelAccessGrantedResponse, KernelAccessGrantsListedResponse, KernelAccessRevokedResponse, KernelAccessStatus, KernelAccessStatusResponse } from "@chariox/kernel-client/kernel-types"
import { CommandFailure, parseFlags, usageOf } from "./agent-command-table.js"
import { reportFailure } from "./kernels-command.js"

// The kernel picks the holder from this process's host ancestry unless --holder-pid names one:
// inside a provider sandbox, pids this process can see are not host pids.
function holderPid(flags: Map<string, string | true>): { holder_pid?: number } {
  if (!flags.has("--holder-pid")) return {}
  const holder = Number(flags.get("--holder-pid"))
  if (!Number.isSafeInteger(holder) || holder <= 1) throw new CommandFailure("usage", "Access requires a positive integer holder pid")
  return { holder_pid: holder }
}

export function parseAccessRequest(argv: string[]) {
  const flags = parseFlags("access request", argv)
  if (flags.has("--until-exit") && flags.has("--minutes")) throw new CommandFailure("usage", "Choose --until-exit or --minutes, not both")
  const minutes = flags.has("--minutes") ? Number(flags.get("--minutes")) : undefined
  if (minutes !== undefined && (!Number.isSafeInteger(minutes) || minutes < 1)) throw new CommandFailure("usage", "Access requires a positive integer lifetime")
  const term = flags.has("--until-exit") ? "until_holder_exits" as const : minutes !== undefined ? { minutes } : undefined
  const noWait = flags.has("--no-wait")
  return {
    request: { RequestKernelAccess: { ...holderPid(flags), ...(term ? { term } : {}), ...(noWait ? { no_wait: true } : {}) } },
    socket: flags.get("--socket") as string | undefined,
    noWait,
  }
}

function printStatus(status: KernelAccessStatus): void {
  const { state, holder_pid, reason } = status
  console.log(JSON.stringify({ status: state, holder_pid, ...(reason ? { reason: reason.code, message: reason.message } : {}) }))
}

async function requestAccess(argv: string[]): Promise<void> {
  const { request, socket, noWait } = parseAccessRequest(argv)
  const client = kernelUnixClient(socket)
  try {
    if (noWait) {
      const { status } = (await client.send<KernelAccessStatusResponse>(request)).KernelAccessStatus
      if (status.state === "granted" && status.grant) {
        console.log(JSON.stringify(status.grant))
        return
      }
      // The kernel waits for the owner and binds the answer to the holder: this process may exit.
      console.log(JSON.stringify({ status: status.state, holder_pid: status.holder_pid, note: "approve in your Chariox client, then run chariox access status" }))
      process.exitCode = 6
      return
    }
    console.error("Waiting for the kernel's access popup.")
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
  const client = kernelUnixClient(flags.get("--socket") as string | undefined)
  try {
    const response = await client.send<KernelAccessStatusResponse>({ GetKernelAccessStatus: holderPid(flags) })
    printStatus(response.KernelAccessStatus.status)
  } finally { await client.close() }
}

const ACCESS_USAGE = [usageOf("access request"), usageOf("access status"), usageOf("access cover"), "chariox access list", "chariox access revoke <id|--all>"].join(" | ")

export async function runAccessCommand(argv: string[]): Promise<boolean> {
  if (argv[0] !== "access") return false
  if (argv[1] === "--help") {
    console.log(ACCESS_USAGE)
    return true
  }
  const direct = { request: requestAccess, status: accessStatus, cover: coverProcess }[argv[1] ?? ""]
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
