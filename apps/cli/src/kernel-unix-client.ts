import { readFileSync } from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"
import { LocalIpcClient } from "@chariox/kernel-client/ipc"

export function kernelUnixClient(socket?: string): LocalIpcClient {
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
  return new LocalIpcClient(`ws+unix://${socketPath}`, { controlRequestRetryDeadlineMs: 0, controlResponseStallMs: 24 * 60 * 60 * 1000 })
}
