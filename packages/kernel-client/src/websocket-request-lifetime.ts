import { setTimeout as sleep } from "node:timers/promises"
import { LocalIpcError } from "./local-ipc-error.js"

// Explicit client shutdown retires outstanding transport work, not requests
// already accepted by the kernel. Later explicit sends may use a new lifetime.
export class KernelRequestLifetime {
  private controller = new AbortController()

  capture(): AbortSignal {
    return this.controller.signal
  }

  retire(message: string): void {
    const retired = this.controller
    this.controller = new AbortController()
    retired.abort(new LocalIpcError("kernel websocket", message, "client_closed", false))
  }
}

export async function waitForKernelRequestReplay(milliseconds: number, signal: AbortSignal): Promise<void> {
  signal.throwIfAborted()
  try {
    await sleep(milliseconds, undefined, { signal })
  } catch (error) {
    signal.throwIfAborted()
    throw error
  }
}

// MP-08 / MP-10 / MP-11: retire one caller's wait without closing the shared
// socket or cancelling work already accepted by the owning kernel.
export function waitForKernelRequest<T>(
  promise: Promise<T>,
  signal: AbortSignal,
  retire?: (error: LocalIpcError) => void,
): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const abort = () => {
      const error = signal.reason instanceof LocalIpcError ? signal.reason
        : new LocalIpcError("kernel request", "request cancelled", "request_cancelled", false)
      retire?.(error)
      signal.removeEventListener("abort", abort)
      reject(error)
    }
    signal.addEventListener("abort", abort, { once: true })
    promise.then(value => {
      signal.removeEventListener("abort", abort)
      resolve(value)
    }, error => {
      signal.removeEventListener("abort", abort)
      reject(error)
    })
    if (signal.aborted) abort()
  })
}
