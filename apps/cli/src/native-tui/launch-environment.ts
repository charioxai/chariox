import { execFile } from "node:child_process"
import net from "node:net"
import process from "node:process"
import { promisify } from "node:util"

const execFileAsync = promisify(execFile)

export function parseNativeMode(value: string): "build" | "plan" {
  if (value === "build" || value === "plan") return value
  throw new Error("--mode must be build or plan")
}

export function parseNativePermissions(value: string): "required" | "yolo" {
  if (value === "required" || value === "yolo") return value
  throw new Error("--permissions must be required or yolo")
}

export async function inferWorkspaceTargetsFromLaunchDirectory(cwd: string): Promise<{ workspace: string; worktree: string }> {
  try {
    const [worktreeResult, commonDirResult] = await Promise.all([
      execFileAsync("git", ["rev-parse", "--show-toplevel"], { cwd }),
      execFileAsync("git", ["rev-parse", "--path-format=absolute", "--git-common-dir"], { cwd }),
    ])
    const worktree = worktreeResult.stdout.trim()
    const commonDir = commonDirResult.stdout.trim()
    if (!worktree) return { workspace: cwd, worktree: cwd }
    const workspace = commonDir.endsWith("/.git")
      ? commonDir.slice(0, -"/.git".length)
      : worktree
    return { workspace, worktree }
  } catch {
    return { workspace: cwd, worktree: cwd }
  }
}

export function defaultKernelEndpoint(kernelPort?: string): string {
  if (process.env.CHARIOX_KERNEL_URL) return process.env.CHARIOX_KERNEL_URL
  const host = process.env.CHARIOX_KERNEL_HOST ?? "127.0.0.1"
  const port = kernelPort ?? process.env.CHARIOX_KERNEL_PORT ?? "43119"
  return `ws://${host}:${port}/kernel`
}

export function parseKernelPort(value: string, arg: string): string {
  if (!/^\d+$/.test(value)) throw new Error(`${arg} must be a TCP port`)
  const port = Number(value)
  if (!Number.isInteger(port) || port < 1 || port > 65535) {
    throw new Error(`${arg} must be between 1 and 65535`)
  }
  return String(port)
}

export async function reserveLocalPort(): Promise<number> {
  return await new Promise((resolve, reject) => {
    const server = net.createServer()
    server.once("error", reject)
    server.listen(0, "127.0.0.1", () => {
      const address = server.address()
      if (!address || typeof address === "string") {
        server.close(() => reject(new Error("port reservation did not expose a TCP address")))
        return
      }
      const port = address.port
      server.close(() => resolve(port))
    })
  })
}

export function shellQuote(value: string): string {
  return `'${String(value).replaceAll("'", "'\\''")}'`
}
