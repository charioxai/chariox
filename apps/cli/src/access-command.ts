import { kernelUnixClient } from "./kernel-unix-client.js"
import { execFileSync } from "node:child_process"
import { LocalIpcClient } from "@chariox/kernel-client/ipc"
import type { KernelAccessGrantedResponse, KernelAccessGrantsListedResponse, KernelAccessRevokedResponse } from "@chariox/kernel-client/kernel-types"

function parentPid(pid: number): number {
  const parent = Number(execFileSync("/bin/ps", ["-o", "ppid=", "-p", String(pid)], { encoding: "utf8" }).trim())
  if (!Number.isSafeInteger(parent) || parent <= 1) throw new Error("Cannot determine holder; provide --holder-pid")
  return parent
}

export function parseAccessRequest(argv: string[], defaultHolder: () => number) {
  const values = new Map<string, string>()
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i]!
    const value = argv[i + 1]
    if (!["--holder-pid", "--minutes", "--socket"].includes(key) || !value || values.has(key)) {
      throw new Error("Usage: chariox access request [--holder-pid <pid>] [--minutes <minutes>] [--socket <absolute-path>]")
    }
    values.set(key, value)
  }
  const holder_pid = values.has("--holder-pid") ? Number(values.get("--holder-pid")) : defaultHolder()
  const lifetime_minutes = values.has("--minutes") ? Number(values.get("--minutes")) : undefined
  if (!Number.isSafeInteger(holder_pid) || holder_pid <= 1
    || (lifetime_minutes !== undefined && (!Number.isSafeInteger(lifetime_minutes) || lifetime_minutes < 1))) {
    throw new Error("Access requires a positive integer holder pid and lifetime")
  }
  return { request: { RequestKernelAccess: { holder_pid, lifetime_minutes } }, socket: values.get("--socket") }
}

export async function runAccessCommand(argv: string[]): Promise<boolean> {
  if (argv[0] !== "access") return false
  if (argv[1] === "request") {
    const { request, socket } = parseAccessRequest(argv.slice(2), () => process.env.CHARIOX_CLI_ACCESS_HOLDER_PID ? Number(process.env.CHARIOX_CLI_ACCESS_HOLDER_PID) : parentPid(parentPid(process.pid)))
    const client = kernelUnixClient(socket)
    try {
      console.error(`Waiting for the kernel's access popup for holder pid ${request.RequestKernelAccess.holder_pid}.`)
      const response = await client.send<KernelAccessGrantedResponse>(request)
      console.log(JSON.stringify(response.KernelAccessGranted.grant))
    } finally { await client.close() }
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
      throw new Error("Usage: chariox access request [--holder-pid <pid>] [--minutes <minutes>] [--socket <path>] | access list | access revoke <id|--all>")
    }
  } finally { await client.close() }
  return true
}
