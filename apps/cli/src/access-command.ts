import { execFileSync } from "node:child_process"
import { readFileSync } from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"
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
    if (!["--session", "--holder-pid", "--minutes", "--socket"].includes(key) || !value || values.has(key)) {
      throw new Error("Usage: chariox access request --session <id> [--holder-pid <pid>] [--minutes <minutes>] [--socket <absolute-path>]")
    }
    values.set(key, value)
  }
  const session_id = values.get("--session")
  const holder_pid = values.has("--holder-pid") ? Number(values.get("--holder-pid")) : defaultHolder()
  const lifetime_minutes = values.has("--minutes") ? Number(values.get("--minutes")) : undefined
  if (!session_id || !Number.isSafeInteger(holder_pid) || holder_pid <= 1
    || (lifetime_minutes !== undefined && (!Number.isSafeInteger(lifetime_minutes) || lifetime_minutes < 1))) {
    throw new Error("Access requires a session id and positive integer holder pid and lifetime")
  }
  return { request: { RequestKernelAccess: { session_id, holder_pid, lifetime_minutes } }, socket: values.get("--socket") }
}

export async function runAccessCommand(argv: string[]): Promise<boolean> {
  if (argv[0] !== "access") return false
  if (argv[1] === "request") {
    const { request, socket } = parseAccessRequest(argv.slice(2), () => process.env.CHARIOX_CLI_ACCESS_HOLDER_PID ? Number(process.env.CHARIOX_CLI_ACCESS_HOLDER_PID) : parentPid(parentPid(process.pid)))
    const charioxHome = process.env.CHARIOX_HOME
    const configRoot = charioxHome || join(process.env.XDG_CONFIG_HOME || homedir(), process.env.XDG_CONFIG_HOME ? "chariox" : ".chariox")
    const runtimeRoot = charioxHome ? join(charioxHome, "run")
      : process.env.XDG_RUNTIME_DIR ? join(process.env.XDG_RUNTIME_DIR, "chariox") : join(homedir(), ".chariox", "run")
    let socketPath = socket ?? process.env.CHARIOX_DAEMON_SOCKET
    if (!socketPath) {
      let kernelId = process.env.CHARIOX_DAEMON_ID
      if (!kernelId) {
        const registry = JSON.parse(readFileSync(join(configRoot, "kernels", "registry.json"), "utf8")) as { kernels?: Record<string, { kernel_id: string }> }
        const endpoint = `${process.env.CHARIOX_KERNEL_HOST || "127.0.0.1"}:${process.env.CHARIOX_KERNEL_PORT || "43118"}`
        kernelId = registry.kernels?.[endpoint]?.kernel_id
      }
      if (!kernelId || kernelId.includes("/") || kernelId === "..") throw new Error("Kernel socket not found; pass --socket <absolute-path>")
      socketPath = join(runtimeRoot, `${kernelId}.sock`)
    }
    if (!socketPath.startsWith("/")) throw new Error("--socket requires an absolute path")
    const client = new LocalIpcClient(`ws+unix://${socketPath}`, { controlRequestRetryDeadlineMs: 0, controlResponseStallMs: 24 * 60 * 60 * 1000 })
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
      console.log(JSON.stringify(result.KernelAccessGrantsListed.grants, null, 2))
    } else if (argv[1] === "revoke" && argv.length === 3) {
      const result = await client.send<KernelAccessRevokedResponse>({ RevokeKernelAccessGrant: { grant_id: argv[2] === "--all" ? null : argv[2] } })
      console.log(`Revoked ${result.KernelAccessRevoked.revoked} grants`)
    } else {
      throw new Error("Usage: chariox access request --session <id> [--holder-pid <pid>] [--minutes <minutes>] [--socket <path>] | access list | access revoke <id|--all>")
    }
  } finally { await client.close() }
  return true
}
