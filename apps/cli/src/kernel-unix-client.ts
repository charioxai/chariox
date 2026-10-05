import { readFileSync } from "node:fs"
import { join } from "node:path"
import { LocalIpcClient } from "@chariox/kernel-client/ipc"
import { defaultKernelUnixSocketPath, kernelSocketConfigRoot } from "@chariox/kernel-client/kernel-unix-socket-path"

export function kernelUnixClient(socket?: string): LocalIpcClient {
  const configRoot = kernelSocketConfigRoot()
  let socketPath = socket ?? process.env.CHARIOX_DAEMON_SOCKET
  if (!socketPath) {
    let kernelId = process.env.CHARIOX_DAEMON_ID
    try {
      const registry = JSON.parse(readFileSync(join(configRoot, "kernels", "registry.json"), "utf8")) as { kernels?: Record<string, { kernel_id: string }> }
      const endpoint = `${process.env.CHARIOX_KERNEL_HOST || "127.0.0.1"}:${process.env.CHARIOX_KERNEL_PORT || "43118"}`
      // The retained endpoint identity also names sockets for scoped slice workers.
      kernelId = registry.kernels?.[endpoint]?.kernel_id || kernelId
    } catch (error) {
      if (!kernelId) throw error
    }
    if (!kernelId || kernelId.includes("/") || kernelId === "..") throw new Error("Kernel socket not found; pass --socket <absolute-path>")
    socketPath = defaultKernelUnixSocketPath(kernelId, configRoot)
  }
  if (!socketPath.startsWith("/")) throw new Error("--socket requires an absolute path")
  return new LocalIpcClient(`ws+unix://${socketPath}`, { controlRequestRetryDeadlineMs: 0, controlResponseStallMs: 24 * 60 * 60 * 1000 })
}
