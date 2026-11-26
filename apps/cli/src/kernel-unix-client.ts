import { readFileSync, readdirSync } from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"
import { LocalIpcClient, type LocalIpcClientOptions } from "@chariox/kernel-client/ipc"
import { defaultKernelUnixSocketPath, kernelSocketConfigRoot } from "@chariox/kernel-client/kernel-unix-socket-path"
import { CommandFailure } from "./agent-command-table.js"
import { loadLocalKernelPresences } from "./local-kernel-presence.js"

type RunningKernel = { readonly kernelId: string; readonly home: string; readonly host: string; readonly port: number }

/** Kernels of this OS user with a fresh heartbeat in the default home and, unless CHARIOX_HOME picks one, in the homes Setup creates. */
function runningKernels(configRoot: string): RunningKernel[] {
  const homes = [configRoot]
  if (!process.env.CHARIOX_HOME) {
    const installs = join(homedir(), ".chariox/dev/machines")
    try {
      for (const entry of readdirSync(installs, { withFileTypes: true }).filter(e => e.isDirectory() && /^[a-z][a-z0-9-]{0,47}$/.test(e.name)).slice(0, 128)) homes.push(join(installs, entry.name))
    } catch { /* no Setup installs */ }
  }
  return homes.flatMap(home => loadLocalKernelPresences(join(home, "kernels/active")).map(presence => ({ kernelId: presence.kernelId, home, host: presence.host, port: presence.port })))
}

/** The kernel that is running now, found by its heartbeat; retained registry identities say nothing about liveness. */
function discoveredKernel(configRoot: string, endpoint: string | undefined): RunningKernel {
  const running = runningKernels(configRoot).filter(kernel => !endpoint || `${kernel.host}:${kernel.port}` === endpoint)
  if (running.length === 1) return running[0]!
  if (running.length === 0) {
    const which = endpoint ? `No running Chariox kernel serves ${endpoint}. Start it, or unset CHARIOX_KERNEL_HOST and CHARIOX_KERNEL_PORT to use the kernel that is running.` : "No Chariox kernel is running for this user. Start it (Chariox Setup with --repair, or chariox-kernel from a checkout), then run this command again."
    throw new CommandFailure("kernel_unreachable", `${which} To use a kernel in another home, set CHARIOX_HOME or pass --socket PATH.`)
  }
  const choices = running.map(kernel => `home ${kernel.home}, port ${kernel.port}: --socket ${defaultKernelUnixSocketPath(kernel.kernelId, kernel.home)} (or CHARIOX_HOME=${kernel.home})`)
  throw new CommandFailure("kernel_ambiguous", `Several Chariox kernels are running here; pick one and run this command again with it:\n${choices.join("\n")}`)
}

export function kernelUnixClient(socket?: string, route: Pick<LocalIpcClientOptions, "kernel" | "commandId" | "onSubscriptionEnded"> = {}): LocalIpcClient {
  const configRoot = kernelSocketConfigRoot()
  let socketPath = socket ?? process.env.CHARIOX_DAEMON_SOCKET
  if (!socketPath) {
    const { CHARIOX_KERNEL_HOST: host, CHARIOX_KERNEL_PORT: port, CHARIOX_HOME: home, CHARIOX_DAEMON_ID: daemonId } = process.env
    const endpoint = host || port ? `${host || "127.0.0.1"}:${port || "43118"}` : undefined
    let kernelId = daemonId
    let kernelHome = configRoot
    // A selection (a home, an endpoint, a scoped worker's id) names the retained identity; the bare case asks who is running.
    if (endpoint || home || daemonId) {
      try {
        const registry = JSON.parse(readFileSync(join(configRoot, "kernels", "registry.json"), "utf8")) as { kernels?: Record<string, { kernel_id: string }> }
        // The retained endpoint identity also names sockets for scoped slice workers.
        kernelId = registry.kernels?.[endpoint ?? "127.0.0.1:43118"]?.kernel_id || kernelId
      } catch { /* nothing retained for this selection: look for a running kernel */ }
    }
    if (!kernelId) ({ kernelId, home: kernelHome } = discoveredKernel(configRoot, endpoint))
    if (kernelId.includes("/") || kernelId === "..") throw new CommandFailure("kernel_unreachable", "The kernel id is not usable. Pass --socket PATH.")
    socketPath = defaultKernelUnixSocketPath(kernelId, kernelHome)
  }
  if (!socketPath.startsWith("/")) throw new Error("--socket requires an absolute path")
  return new LocalIpcClient(`ws+unix://${socketPath}`, { ...route, controlRequestRetryDeadlineMs: 0, controlResponseStallMs: 24 * 60 * 60 * 1000 })
}
