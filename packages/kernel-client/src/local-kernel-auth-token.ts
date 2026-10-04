import { closeSync, constants as fsConstants, fstatSync, openSync, readFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"

// A laptop kernel writes a fresh token at each start to
// `<state dir>/kernel-local-auth/<port>.token` (owner-only), and accepts it on
// the websocket handshake. The state directory mirrors the kernel's
// `default_state_dir`: `$CHARIOX_HOME/state`, `$XDG_STATE_HOME/chariox`, or
// `$HOME/.local/state/chariox`.
const KERNEL_LOCAL_AUTH_DIRECTORY = "kernel-local-auth"
const MAX_KERNEL_LOCAL_AUTH_TOKEN_BYTES = 8 * 1024
const LOOPBACK_HOSTNAMES = new Set(["127.0.0.1", "[::1]", "localhost"])

/** The token file for a loopback kernel endpoint, or null for any other endpoint. */
export function localKernelAuthTokenPath(
  endpoint: string,
  environment: NodeJS.ProcessEnv = process.env,
): string | null {
  const port = loopbackKernelPort(endpoint)
  if (port === null) return null
  return join(kernelStateDirectory(environment), KERNEL_LOCAL_AUTH_DIRECTORY, `${port}.token`)
}

/**
 * Reads the token the local kernel on this endpoint's port wrote. Returns null
 * when there is none or the file is not a private regular file of this user;
 * enforcement refuses such connections with a diagnostic. Never throws.
 */
export function readLocalKernelAuthToken(endpoint: string, environment: NodeJS.ProcessEnv = process.env): string | null {
  const path = localKernelAuthTokenPath(endpoint, environment)
  if (!path) return null
  let descriptor: number
  try {
    descriptor = openSync(path, fsConstants.O_RDONLY | fsConstants.O_NOFOLLOW | fsConstants.O_NONBLOCK)
  } catch {
    return null
  }
  try {
    const metadata = fstatSync(descriptor)
    const currentUid = process.getuid?.()
    if (
      !metadata.isFile()
      || (metadata.mode & 0o077) !== 0
      || (currentUid !== undefined && metadata.uid !== currentUid)
      || metadata.size > MAX_KERNEL_LOCAL_AUTH_TOKEN_BYTES
    ) {
      return null
    }
    const token = readFileSync(descriptor, "utf8").trim()
    return /^[\x21-\x7e]+$/.test(token) ? token : null
  } catch {
    return null
  } finally {
    closeSync(descriptor)
  }
}

function loopbackKernelPort(endpoint: string): number | null {
  let url: URL
  try {
    url = new URL(endpoint)
  } catch {
    return null
  }
  if (url.protocol !== "ws:" || !LOOPBACK_HOSTNAMES.has(url.hostname) || url.username || url.password) {
    return null
  }
  return url.port ? Number(url.port) : 80
}

function kernelStateDirectory(environment: NodeJS.ProcessEnv): string {
  if (environment.CHARIOX_HOME) return join(environment.CHARIOX_HOME, "state")
  if (environment.XDG_STATE_HOME) return join(environment.XDG_STATE_HOME, "chariox")
  if (environment.HOME) return join(environment.HOME, ".local", "state", "chariox")
  return join(tmpdir(), "chariox")
}
