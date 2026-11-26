import { readFileSync, readdirSync } from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"
import { LocalIpcClient, type LocalIpcClientOptions } from "@chariox/kernel-client/ipc"
import { defaultKernelUnixSocketPath, kernelSocketConfigRoot } from "@chariox/kernel-client/kernel-unix-socket-path"
import { CommandFailure } from "./agent-command-table.js"
import { loadLocalKernelPresences } from "./local-kernel-presence.js"

type RunningKernel = { readonly kernelId: string; readonly home: string }

/** Kernels with a fresh heartbeat in the default home and, unless CHARIOX_HOME picks one, in the homes Setup creates. */
function runningKernels(configRoot: string): RunningKernel[] {
  const homes = [configRoot]
  if (!process.env.CHARIOX_HOME) {
    const installs = join(homedir(), ".chariox/dev/machines")
    try {
      for (const entry of readdirSync(installs, { withFileTypes: true }).filter(e => e.isDirectory() && /^[a-z][a-z0-9-]{0,47}$/.test(e.name)).slice(0, 128)) homes.push(join(installs, entry.name))
    } catch { /* no Setup installs */ }
  }
  return homes.flatMap(home => loadLocalKernelPresences(join(home, "kernels/active")).map(presence => ({ kernelId: presence.kernelId, home })))
}

/** Without an explicit socket, find the one local kernel; say what to do when there is none or several. */
function discoveredKernel(configRoot: string): RunningKernel {
  const running = runningKernels(configRoot)
  if (running.length === 1) return running[0]!
  if (running.length === 0) throw new CommandFailure("kernel_unreachable", "No Chariox kernel is running for this user. Start it (Chariox Setup with --repair, or chariox-kernel from a checkout), then run this command again. To use a kernel in another home, set CHARIOX_HOME or pass --socket PATH.")
  const homes = running.map(kernel => kernel.home).join(", ")
  throw new CommandFailure("kernel_ambiguous", `Several Chariox kernels are running here (homes: ${homes}). Set CHARIOX_HOME to the home of the one you want, or pass --socket PATH.`)
}

export function kernelUnixClient(socket?: string, route: Pick<LocalIpcClientOptions, "kernel" | "commandId" | "onSubscriptionEnded"> = {}): LocalIpcClient {
  const configRoot = kernelSocketConfigRoot()
  let socketPath = socket ?? process.env.CHARIOX_DAEMON_SOCKET
  if (!socketPath) {
    let kernelId = process.env.CHARIOX_DAEMON_ID
    let home = configRoot
    try {
      const registry = JSON.parse(readFileSync(join(configRoot, "kernels", "registry.json"), "utf8")) as { kernels?: Record<string, { kernel_id: string }> }
      const endpoint = `${process.env.CHARIOX_KERNEL_HOST || "127.0.0.1"}:${process.env.CHARIOX_KERNEL_PORT || "43118"}`
      // The retained endpoint identity also names sockets for scoped slice workers.
      kernelId = registry.kernels?.[endpoint]?.kernel_id || kernelId
    } catch { /* no registry for the default endpoint: look for a running kernel */ }
    if (!kernelId) ({ kernelId, home } = discoveredKernel(configRoot))
    if (kernelId.includes("/") || kernelId === "..") throw new CommandFailure("kernel_unreachable", "The kernel id is not usable. Pass --socket PATH.")
    socketPath = defaultKernelUnixSocketPath(kernelId, home)
  }
  if (!socketPath.startsWith("/")) throw new Error("--socket requires an absolute path")
  return new LocalIpcClient(`ws+unix://${socketPath}`, { ...route, controlRequestRetryDeadlineMs: 0, controlResponseStallMs: 24 * 60 * 60 * 1000 })
}
