export type RenewedRelayGrant = { token: string; expiresAtMs: number }
export type RelayGrantProvider = () => Promise<RenewedRelayGrant>

const retryBaseMs = 1_000
const retryMaxMs = 60_000
const retryJitterMs = 1_000

/** The issuer says when to come back ("retry in 20 seconds"); never earlier. */
export function relayRetryHintMs(error: unknown): number {
  const message = error instanceof Error ? error.message : String(error)
  const seconds = /retry in (\d{1,5}) seconds?/i.exec(message)?.[1]
  return seconds ? Number(seconds) * 1_000 : 0
}

/** Jittered exponential backoff, capped, never shorter than the issuer's hint. */
export function relayRenewalRetryDelayMs(failures: number, error: unknown, random: () => number = Math.random): number {
  const backoff = Math.min(retryMaxMs, retryBaseMs * 2 ** Math.min(failures, 6))
  return Math.max(backoff, Math.min(relayRetryHintMs(error), 3_600_000)) + Math.floor(random() * retryJitterMs)
}

/** One renewal at a time, before expiry. Transport faults retry silently;
 * revocation ends the connection rather than asking for periodic login. */
export class RelayAuthRenewal {
  private timer: ReturnType<typeof setTimeout> | undefined
  private pending: Promise<void> | undefined
  private stopped = false
  private terminalError: unknown
  private retries = 0
  private retryAt = 0
  private lastFailure: unknown
  private tokenRefused = false
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
    if (this.stopped) return
    // While the issuer is backing us off, a connect attempt must not ask again
    // and must not fall back to the token that was refused.
    if (this.lastFailure !== undefined && Date.now() < this.retryAt) throw this.lastFailure
    if (this.tokenRefused || Date.now() >= this.renewAt) await this.refresh()
  }
  /** The relay refused the current token as expired: it is not presented again before a renewed one replaces it. */
  refused(): void {
    if (this.stopped) return
    this.tokenRefused = true
    if (Date.now() < this.retryAt) return
    void this.refresh().catch(() => {})
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
        this.retryAt = 0
        this.lastFailure = undefined
        this.tokenRefused = false
        this.schedule()
      } catch (error) {
        const code = (error as {code?: string})?.code
        if (["client_revoked", "refresh_reuse_detected", "identity_revoked", "authorization_denied"].includes(code ?? "")) {
          this.invalidate(error)
        } else {
          const delay = relayRenewalRetryDelayMs(this.retries++, error)
          this.lastFailure = error
          this.retryAt = Date.now() + delay
          this.schedule(delay)
        }
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
