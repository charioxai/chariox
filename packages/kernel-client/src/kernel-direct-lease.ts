import { LocalDirectSecurityError } from "./kernel-direct-security.js"

/** MP-08/MP-11: a local socket cannot keep itself alive without relay renewal. */
export const localLeaseRenewalIntervalMs = 10_000
/** Transient failures retry within the existing kernel-issued lease. */
export const localLeaseRetryDelayMs = 1_000

type Timer = ReturnType<typeof setTimeout> | number

export type LocalBrowserLease = { readonly socket: WebSocket; expiresAtMs: number }

/**
 * MP-08/MP-11: renews every local socket of one kernel together, so a tab
 * spends one Cloud lease token and one relay connection per interval however
 * many lanes and clients use the local carrier.
 */
export class LocalBrowserLeaseKeeper<T extends LocalBrowserLease> {
  private readonly leases = new Set<T>()
  private closed = false
  private active = false
  private timer: Timer | undefined

  constructor(private readonly deps: {
    /** Renews the batch and returns the leases the kernel refused. */
    readonly renew: (leases: readonly T[]) => Promise<readonly T[]>
    readonly consented: () => boolean
    readonly nowMs?: () => number
    readonly onFailure: () => void
    readonly setTimeout?: (callback: () => void, delayMs: number) => Timer
    readonly clearTimeout?: (timer: Timer) => void
  }) {}

  add(lease: T): void {
    if (this.closed) { lease.socket.close(); return }
    this.leases.add(lease)
    lease.socket.addEventListener("close", () => this.leases.delete(lease), { once: true })
    if (!this.active) this.schedule(localLeaseRenewalIntervalMs)
  }

  close(): void {
    this.closed = true
    if (this.timer !== undefined) (this.deps.clearTimeout ?? clearTimeout)(this.timer as ReturnType<typeof setTimeout>)
    this.timer = undefined
    this.retire([...this.leases], "direct client closed")
  }

  private schedule(delayMs: number): void {
    if (this.closed) return
    this.active = true
    this.timer = (this.deps.setTimeout ?? setTimeout)(() => { this.timer = undefined; void this.renew() }, delayMs)
  }

  private async renew(): Promise<void> {
    const leases = [...this.leases].filter((lease) => lease.socket.readyState === 1)
    if (leases.length === 0) {
      this.active = false
      return
    }
    if (leases.some((lease) => lease.expiresAtMs <= this.nowMs())) {
      this.fail(leases)
      this.active = false
      return
    }
    if (!this.deps.consented()) {
      this.retire(leases, "local connection consent withdrawn")
      this.active = false
      return
    }
    try {
      const refused = await this.deps.renew(leases)
      if (refused.length > 0) this.fail(refused)
    } catch (error) {
      const live = leases.filter((lease) => lease.socket.readyState === 1)
      const remainingMs = Math.min(...live.map((lease) => lease.expiresAtMs)) - this.nowMs()
      if (live.length > 0 && remainingMs > 0 && !(error instanceof LocalDirectSecurityError)) {
        this.schedule(Math.min(localLeaseRetryDelayMs, remainingMs))
        return
      }
      if (live.length > 0) this.fail(live)
    }
    if (this.leases.size > 0) this.schedule(localLeaseRenewalIntervalMs)
    else this.active = false
  }

  private nowMs(): number { return this.deps.nowMs?.() ?? Date.now() }

  private fail(leases: readonly T[]): void {
    this.retire(leases, "local browser lease renewal failed")
    this.deps.onFailure()
  }

  private retire(leases: readonly T[], reason: string): void {
    for (const lease of leases) {
      this.leases.delete(lease)
      lease.socket.close(1000, reason)
    }
  }
}
