// MP-08/MP-11: refresh through the existing authenticated kernel issuer on
// the relay. Decoded claims schedule/check scope; they never authenticate.
type Claims = Record<string, unknown> & { exp: number }

function claims(token: string): Claims | null {
  try {
    const payload = token.split(".")[1]
    if (!payload || payload.length > 32_768) return null
    const value = JSON.parse(Buffer.from(payload, "base64url").toString("utf8")) as Claims
    return Number.isFinite(value.exp) ? value : null
  } catch { return null }
}

export class TerminalRelayIdentity {
  private value: string
  private key: string | null = null
  private timer: NodeJS.Timeout | null = null
  private closed = false
  private refreshing = false

  constructor(private readonly deps: {
    token: string; relayUrl: string; kernelId: string; thumbprint: string
    eligible(): boolean
    request(key: string, body: unknown): Promise<Record<string, unknown>>
    onToken(token: string): void
  }) { this.value = deps.token }

  token(): string { return this.value }

  start(key: string): void { this.key = key; this.schedule() }

  close(): void {
    this.closed = true
    if (this.timer) clearTimeout(this.timer)
    this.timer = null
  }

  private schedule(retry = false): void {
    const current = claims(this.value)
    if (this.closed || this.refreshing || this.timer || !this.key || !current || current.exp * 1000 <= Date.now()) return
    const delay = retry ? 5_000 : Math.max(1_000, current.exp * 1000 - Date.now() - 60_000)
    this.timer = setTimeout(() => { this.timer = null; void this.refresh() }, Math.min(delay, 2_147_483_647))
    this.timer.unref?.()
  }

  private async refresh(): Promise<void> {
    const current = claims(this.value)
    const key = this.key
    if (this.closed || !key || !current || current.exp * 1000 <= Date.now()) return
    this.refreshing = true
    let failed = true
    try {
      if (!this.deps.eligible() || current.subject_kind !== "client" || current.public_key_thumbprint !== this.deps.thumbprint) throw Error("terminal identity is unavailable")
      const body = await this.deps.request(key, { IssueCloudRelayClientToken: {
        target_daemon_alias: this.deps.kernelId,
        client_id: current.client_id ?? current.sub,
        session_id: current.session_id ?? null,
        public_key_thumbprint: this.deps.thumbprint,
      } })
      if (this.closed || this.key !== key) return
      const issued = body.CloudRelayClientTokenIssued as { token?: { relay_token?: unknown; relay_url?: unknown } } | undefined
      const token = issued?.token?.relay_token
      if (typeof token !== "string" || typeof issued?.token?.relay_url !== "string"
        || new URL(issued.token.relay_url).href !== new URL(this.deps.relayUrl).href) throw Error("terminal identity issuer changed")
      const next = claims(token)
      const principal = ["iss", "subject_kind", "realm_id", "account_id", "user_id", "public_key_thumbprint", "machine_id", "session_id"]
      if (!next || next.exp * 1000 <= Date.now() + 60_000
        || principal.some(field => (next[field] ?? null) !== (current[field] ?? null))
        || !Array.isArray(next.allowed_targets) || next.allowed_targets.length !== 1 || next.allowed_targets[0] !== this.deps.kernelId
        || !Array.isArray(next.allowed_actions) || !Array.isArray(current.allowed_actions)
        || next.allowed_actions.some(action => !(current.allowed_actions as unknown[]).includes(action))) throw Error("terminal identity scope changed")
      this.value = token
      this.deps.onToken(token)
      failed = false
    } catch { /* Retry only while the current identity still authorizes it. */ }
    finally { this.refreshing = false; this.schedule(failed) }
  }
}
