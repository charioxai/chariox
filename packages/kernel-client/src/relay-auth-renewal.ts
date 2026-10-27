export type RenewedRelayGrant = { token: string; expiresAtMs: number }
export type RelayGrantProvider = () => Promise<RenewedRelayGrant>

/** One renewal at a time, before expiry. Transport faults retry silently;
 * revocation ends the connection rather than asking for periodic login. */
export class RelayAuthRenewal {
  private timer: ReturnType<typeof setTimeout> | undefined
  private pending: Promise<void> | undefined
  private stopped = false
  private terminalError: unknown
  private retries = 0
  private renewAt: number
  constructor(private expiry: number, private readonly issue: RelayGrantProvider,
    private readonly apply: (grant: RenewedRelayGrant) => void | Promise<void>,
    private readonly revoked: () => void,
    private readonly release?: () => void) { this.renewAt = this.nextRenewal(); this.schedule() }
  private nextRenewal() { return this.expiry - Math.min(60_000, Math.max(10, (this.expiry - Date.now()) / 4)) }
  private schedule(delay?: number) {
    if (this.stopped) return
    this.timer = setTimeout(() => { void this.refresh().catch(() => {}) }, delay ?? Math.max(0, this.renewAt - Date.now()))
    this.timer.unref?.()
  }
  async ensureFresh(): Promise<void> {
    this.checkAuthorization()
    if (!this.stopped && Date.now() >= this.renewAt) await this.refresh()
  }
  checkAuthorization(): void {
    if (this.terminalError) throw this.terminalError
  }
  invalidate(error: unknown): void {
    this.terminalError = error
    this.stop()
    this.revoked()
  }
  refresh(): Promise<void> {
    if (this.stopped) return Promise.resolve()
    if (this.pending) return this.pending
    if (this.timer) clearTimeout(this.timer)
    this.pending = Promise.resolve().then(async () => {
      try {
        const grant = await this.issue()
        if (this.stopped) return
        if (!grant.token || !Number.isFinite(grant.expiresAtMs) || grant.expiresAtMs <= Date.now()) throw new Error("renewed relay grant is already expired")
        await this.apply(grant)
        if (this.stopped) return
        this.expiry = grant.expiresAtMs
        this.renewAt = this.nextRenewal()
        this.retries = 0
        this.schedule()
      } catch (error) {
        const code = (error as {code?: string})?.code
        if (["client_revoked", "refresh_reuse_detected", "identity_revoked", "authorization_denied"].includes(code ?? "")) {
          this.invalidate(error)
        } else this.schedule(Math.min(5_000, 250 * 2 ** Math.min(this.retries++, 5)))
        throw error
      }
    }).finally(() => { this.pending = undefined })
    return this.pending
  }
  stop() {
    if (this.stopped) return
    this.stopped = true
    if (this.timer) clearTimeout(this.timer)
    this.release?.()
  }
}
