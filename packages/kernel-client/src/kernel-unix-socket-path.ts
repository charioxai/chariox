import { createHash } from "node:crypto"
import { join, resolve } from "node:path"

export function kernelSocketConfigRoot(environment: NodeJS.ProcessEnv = process.env): string {
  if (environment.CHARIOX_HOME) return environment.CHARIOX_HOME
  if (environment.XDG_CONFIG_HOME) return join(environment.XDG_CONFIG_HOME, "chariox")
  if (environment.HOME) return join(environment.HOME, ".chariox")
  return join(environment.TMPDIR || "/tmp", "chariox", "config")
}

/** Matches DaemonConfig: private UID directory, full SHA-256 of home and ID. */
export function defaultKernelUnixSocketPath(
  kernelId: string,
  configRoot = kernelSocketConfigRoot(),
  uid = process.geteuid?.(),
): string {
  if (uid === undefined) throw new Error("Kernel Unix access requires a Unix OS user")
  const hash = createHash("sha256").update("chariox-unix-socket-v1\0")
    .update(resolve(configRoot)).update("\0").update(kernelId).digest("hex")
  return `/tmp/chariox-${uid}/${hash}.sock`
}
