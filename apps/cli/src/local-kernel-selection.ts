import type { LocalIpcClient } from "./ipc.js"
import {
  loadLocalKernelPresences,
  localKernelEndpoint,
  type LocalKernelPresence,
} from "./local-kernel-presence.js"
import { getRelayStatus } from "./relay-api.js"

const localKernelProbeTimeoutMs = 2_000

export type LocalKernelTarget = {
  readonly kernelId?: string | null | undefined
}

export type LocalKernelSelection = {
  readonly client: LocalIpcClient
  readonly endpoint: string
  readonly presence: LocalKernelPresence
}

/**
 * MP-08: when a relay-addressed kernel runs on this machine, use its local
 * authenticated endpoint instead of the relay hop. The presence file is only a
 * hint; the kernel must answer as the requested kernel over its owner-only
 * local credential, otherwise the caller keeps using the relay.
 */
export async function selectLocalKernelClient(
  target: LocalKernelTarget,
  createClient: (endpoint: string) => LocalIpcClient,
  presences: () => LocalKernelPresence[] = loadLocalKernelPresences,
): Promise<LocalKernelSelection | null> {
  const kernelId = target.kernelId?.trim()
  // MP-08/MP-11: aliases are not unique across relay realms.
  if (!kernelId) return null
  const presence = presences().find((candidate) => (
    candidate.kernelId === kernelId
  ))
  if (!presence) {
    return null
  }
  const endpoint = localKernelEndpoint(presence)
  const client = createClient(endpoint)
  try {
    const status = await withTimeout(getRelayStatus(client), localKernelProbeTimeoutMs)
    if (status.daemon_id === kernelId) {
      return { client, endpoint, presence }
    }
  } catch {
    // Unreachable, unauthenticated or slow: the relay remains the path.
  }
  await Promise.resolve(client.close()).catch(() => {})
  return null
}

function withTimeout<T>(promise: Promise<T>, timeoutMs: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined
  return Promise.race([
    promise,
    new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error("local kernel probe timed out")), timeoutMs)
    }),
  ]).finally(() => clearTimeout(timer))
}
