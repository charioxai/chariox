/** MP-08 / MP-11: kernel time anchored to a monotonic client clock. */
export type KernelClock = {
  now(): number | null
  synchronize(): Promise<void>
  /** Protocol 538 feed sample, anchored at receipt without local wall-clock drift. */
  observe?(serverNowMs: number): void
  invalidate(): void
}

export function kernelTimeFromHealth(response: unknown): number {
  const value = (response as { DaemonHealth?: { projection?: { metadata?: { generated_at_ms?: unknown } } } })?.DaemonHealth?.projection?.metadata?.generated_at_ms
  if (typeof value !== "number" || !Number.isFinite(value) || value < 0) throw new Error("Kernel time is unavailable")
  return value
}

export function createKernelClock(read: () => Promise<number>, monotonicNow: () => number = () => performance.now()): KernelClock {
  let anchor: { server: number; local: number } | null = null
  let pending: object | null = null
  return {
    now: () => anchor ? anchor.server + monotonicNow() - anchor.local : null,
    invalidate() { anchor = null; pending = null },
    observe(serverNowMs) {
      if (!Number.isFinite(serverNowMs) || serverNowMs < 0) return
      pending = null // A late health response must not replace the newer feed sample.
      anchor = {server: serverNowMs, local: monotonicNow()}
    },
    async synchronize() {
      if (pending) return
      const request = {}
      pending = request
      const started = monotonicNow()
      try {
        const server = await read()
        const finished = monotonicNow()
        if (!Number.isFinite(server) || server < 0) throw new Error("Kernel time is unavailable")
        if (pending === request) anchor = { server: server + (finished - started) / 2, local: finished }
      } finally {
        if (pending === request) pending = null
      }
    },
  }
}

export function approvalCountdown(remainingMs: number | null | undefined): string {
  if (remainingMs == null) return "Checking expiry…"
  const seconds = Math.max(0, Math.ceil(remainingMs / 1_000))
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")} left`
}

export function approvalExpiredMessage(kind: string): string {
  if (kind === "access_extension") return "Extension request expired — access ended"
  if (kind === "access_grant") return "Access request expired"
  if (kind === "sudo") return "Sudo request expired"
  return "Approval request expired"
}
