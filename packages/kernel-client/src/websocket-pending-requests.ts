import type { KernelSocketLane } from "./kernel-transport-frames.js"
import type { EncryptedRelayPayload } from "./kernel-transport-frames.js"
import { LocalIpcError } from "./local-ipc-error.js"

export type PendingKernelRequest = {
  resolve: (value: unknown) => void
  reject: (error: LocalIpcError) => void
  timeout: NodeJS.Timeout | null
  relayDecryptResponse: ((payload: EncryptedRelayPayload) => string) | null
  lane: KernelSocketLane
  stallTimeoutMs: number
  deadlineMs: number
}

export type RegisteredKernelRequest<TResponse> = {
  readonly promise: Promise<TResponse>
  readonly setRelayDecryptResponse: (decryptResponse: (payload: EncryptedRelayPayload) => string) => void
  readonly reject: (error: LocalIpcError) => void
}

export class KernelPendingRequestRegistry {
  private readonly pending = new Map<string, PendingKernelRequest>()

  constructor(private readonly timeoutMs: number) {}

  register<TResponse>(
    requestId: string,
    lane: KernelSocketLane,
    timeoutMs = this.timeoutMs,
    maximumWaitMs = timeoutMs,
  ): RegisteredKernelRequest<TResponse> {
    const promise = new Promise<TResponse>((resolve, reject) => {
      // Zero disables the response timer for kernel-owned authorization waits.
      const pending: PendingKernelRequest = {
        resolve: resolve as (value: unknown) => void,
        reject,
        timeout: null,
        relayDecryptResponse: null,
        lane,
        stallTimeoutMs: timeoutMs,
        deadlineMs: Date.now() + Math.max(timeoutMs, maximumWaitMs),
      }
      this.pending.set(requestId, pending)
      if (timeoutMs !== 0) this.armTimeout(requestId, pending)
    })

    return {
      promise,
      setRelayDecryptResponse: (decryptResponse) => {
        const pending = this.pending.get(requestId)
        if (pending) {
          pending.relayDecryptResponse = decryptResponse
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

  // MP-08 / MP-10 / MP-11: chunks precede encrypted request correlation.
  // Validated progress refreshes only this socket lane's stall timers, never
  // the absolute request deadline or kernel-owned authorization waits.
  noteTransportProgress(lane: KernelSocketLane): void {
    for (const [requestId, pending] of this.pending) {
      if (pending.lane === lane && pending.stallTimeoutMs !== 0) {
        if (pending.timeout) clearTimeout(pending.timeout)
        this.armTimeout(requestId, pending)
      }
    }
  }

  private armTimeout(requestId: string, pending: PendingKernelRequest): void {
    const remainingMs = Math.max(0, pending.deadlineMs - Date.now())
    pending.timeout = setTimeout(() => {
      if (this.pending.get(requestId) !== pending) return
      this.pending.delete(requestId)
      pending.reject(new LocalIpcError("handle kernel response", "timed out", "request_timeout", true))
    }, Math.min(pending.stallTimeoutMs, remainingMs))
  }

  take(requestId: string): PendingKernelRequest | null {
    const pending = this.pending.get(requestId)
    if (!pending) {
      return null
    }
    if (pending.timeout) clearTimeout(pending.timeout)
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
      if (pending.timeout) clearTimeout(pending.timeout)
      pending.reject(new LocalIpcError("kernel websocket", message, "connection_closed", true))
    }
  }
}
