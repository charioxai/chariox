import type { KernelSocketLane } from "./kernel-transport-frames.js"
import { LocalIpcError } from "./local-ipc-error.js"

export type PendingKernelRequest = {
  resolve: (value: unknown) => void
  reject: (error: LocalIpcError) => void
  timeout: NodeJS.Timeout
  relayPrivateKey: Buffer | null
  lane: KernelSocketLane
}

export type RegisteredKernelRequest<TResponse> = {
  readonly promise: Promise<TResponse>
  readonly setRelayPrivateKey: (privateKey: Buffer | null) => void
  readonly reject: (error: LocalIpcError) => void
}

export class KernelPendingRequestRegistry {
  private readonly pending = new Map<string, PendingKernelRequest>()

  constructor(private readonly timeoutMs: number) {}

  register<TResponse>(
    requestId: string,
    lane: KernelSocketLane,
    timeoutMs = this.timeoutMs,
  ): RegisteredKernelRequest<TResponse> {
    const promise = new Promise<TResponse>((resolve, reject) => {
      const timeout = setTimeout(() => {
        const pending = this.pending.get(requestId)
        if (!pending) {
          return
        }
        this.pending.delete(requestId)
        pending.reject(new LocalIpcError("handle kernel response", "timed out", "request_timeout", true))
      }, timeoutMs)

      this.pending.set(requestId, {
        resolve: resolve as (value: unknown) => void,
        reject,
        timeout,
        relayPrivateKey: null,
        lane,
      })
    })

    return {
      promise,
      setRelayPrivateKey: (privateKey) => {
        const pending = this.pending.get(requestId)
        if (pending) {
          pending.relayPrivateKey = privateKey
        }
      },
      reject: (error) => {
        const pending = this.take(requestId)
        if (pending) {
          pending.reject(error)
        }
      },
    }
  }

  take(requestId: string): PendingKernelRequest | null {
    const pending = this.pending.get(requestId)
    if (!pending) {
      return null
    }
    clearTimeout(pending.timeout)
    this.pending.delete(requestId)
    return pending
  }

  rejectMatching(message: string, lane?: KernelSocketLane): void {
    const pendingEntries = Array.from(this.pending.entries())
      .filter(([, pending]) => !lane || pending.lane === lane)
    for (const [requestId] of pendingEntries) {
      this.pending.delete(requestId)
    }
    for (const [, pending] of pendingEntries) {
      clearTimeout(pending.timeout)
      pending.reject(new LocalIpcError("kernel websocket", message, "connection_closed", true))
    }
  }
}
