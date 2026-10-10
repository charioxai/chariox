import type { KernelBrowserCommand, UserDomainGrant, UserDomainGrantEvent, UserDomainResource } from "./kernel-types-kernel-browser.js"

export const userDomainAccessMinimumProtocol = 443
// LocalIpcClient has a 5s response-stall watchdog and no per-request options.
// Keep each owner observation below that watchdog on every client transport.
export const userDomainGrantPollWaitMs = 1000
export type UserDomainAccessClient = {
  readonly localDaemonProtocolVersion?: number
  send<T>(request: unknown, options?: { signal?: AbortSignal; timeoutMs?: number; retryOnTimeout?: boolean; beforeSend?: () => void }): Promise<T>
}
export function userDomainResourceLabel(resource: UserDomainResource): string {
  switch (resource.kind) {
    case "browser_tab": return `tab ${resource.tab_id}`
    case "app_view": return `App ${resource.view_id}`
    case "note": return `note ${resource.note_id}`
    case "capture": return `capture ${resource.capture_id}`
  }
}
export function userDomainUseNotice(notice: UserDomainGrantEvent["notice"]): string | null {
  return notice ? `Agent ${notice.agent_id} used retained access to ${userDomainResourceLabel(notice.resource)} while not focused.` : null
}
export function userDomainGrantExpiry(grant: UserDomainGrant): string {
  if (grant.expires_at_ms !== undefined && grant.expires_at_ms !== null) {
    const deadline = grant.idle_since_ms === null ? grant.expires_at_ms
      : Math.min(grant.expires_at_ms, grant.idle_since_ms + grant.idle_timeout_seconds * 1000)
    return new Date(deadline).toISOString()
  }
  return grant.idle_since_ms === null ? "Retained during work or pending wake"
    : new Date(grant.idle_since_ms + grant.idle_timeout_seconds * 1000).toISOString()
}

/** Owner-terminal projection only. All admission and expiry decisions stay in the kernel. */
export class UserDomainAccessController {
  snapshot: UserDomainGrantEvent | null = null
  error: string | null = null
  busy = false
  private client: UserDomainAccessClient | null = null
  private binding = ""
  private protocol: number | undefined
  private abort: AbortController | null = null
  private retry: ReturnType<typeof setTimeout> | null = null
  private revision = 0
  private readonly listeners = new Set<() => void>()
  private readonly uses = new Map<string, { since: number; at: number }>()
  constructor(private readonly deps: { client(): UserDomainAccessClient | null; bindingKey(): string }) {}
  subscribe(listener: () => void): () => void { this.listeners.add(listener); return () => this.listeners.delete(listener) }
  private publish(): void { for (const listener of this.listeners) listener() }
  lastObservedRetainedUse(grant: UserDomainGrant): number | null {
    const use = this.uses.get(grant.agent_id)
    return use?.since === grant.since_ms ? use.at : null
  }
  private check(revision: number): void {
    if (revision !== this.revision || this.client !== this.deps.client() || this.binding !== this.deps.bindingKey() || this.client?.localDaemonProtocolVersion !== this.protocol) {
      throw new Error("Kernel or owner changed; refresh access.")
    }
  }
  private async request(command: KernelBrowserCommand, revision: number, signal?: AbortSignal): Promise<UserDomainGrantEvent> {
    this.check(revision)
    const response = await this.client!.send<{ KernelBrowser?: { result?: UserDomainGrantEvent }; Error?: { message?: string } }>(
      { KernelBrowser: { command } }, { ...(signal ? { signal } : {}), timeoutMs: 35000, retryOnTimeout: false, beforeSend: () => this.check(revision) })
    this.check(revision)
    const event = response.KernelBrowser?.result
    if (!event || event.event !== "user_domain_grants_changed" || !Number.isSafeInteger(event.cursor) || event.cursor < 0 || !Array.isArray(event.grants)) {
      throw new Error(response.Error?.message ?? "Invalid access snapshot from kernel.")
    }
    return event
  }
  private apply(event: UserDomainGrantEvent): void {
    if (this.snapshot && event.cursor < this.snapshot.cursor) return
    // A notice can outlive a revoked grant. Never present it as current authority.
    const notice = event.notice && event.grants.some(g => g.agent_id === event.notice!.agent_id && g.since_ms <= event.notice!.at_ms)
      ? event.notice : null
    this.snapshot = { ...event, notice }
    for (const [id, use] of this.uses) {
      if (!event.grants.some(g => g.agent_id === id && g.since_ms === use.since)) this.uses.delete(id)
    }
    if (notice) {
      const grant = event.grants.find(g => g.agent_id === notice.agent_id)!
      this.uses.set(grant.agent_id, { since: grant.since_ms, at: notice.at_ms })
    }
    this.error = null; this.publish()
  }
  /** Call on connection/owner changes; starts one cursor feed even when the access view is closed. */
  sync(): void {
    const client = this.deps.client(), binding = this.deps.bindingKey()
    if (client === this.client && binding === this.binding && client?.localDaemonProtocolVersion === this.protocol) return
    this.stop()
    this.client = client; this.binding = binding; this.protocol = client?.localDaemonProtocolVersion
    if (!client || (client.localDaemonProtocolVersion ?? 0) < userDomainAccessMinimumProtocol) {
      this.error = "Update the connected kernel for access grants (protocol 443)."; this.publish(); return
    }
    this.abort = new AbortController()
    void this.poll(this.revision, this.abort.signal)
  }
  stop(): void {
    ++this.revision; this.abort?.abort(); this.abort = null
    if (this.retry !== null) clearTimeout(this.retry)
    this.retry = null
    this.client = null; this.binding = ""; this.protocol = undefined; this.snapshot = null; this.uses.clear(); this.busy = false; this.error = null
    this.publish()
  }
  refresh(): void { this.stop(); this.sync() }
  private async poll(revision: number, signal: AbortSignal): Promise<void> {
    try {
      this.apply(await this.request({ op: "list_grants" }, revision, signal))
      while (!signal.aborted && revision === this.revision) {
        const after = this.snapshot!.cursor
        const event = await this.request({ op: "subscribe_grants", after, wait_ms: userDomainGrantPollWaitMs }, revision, signal)
        // IPC may hide a kernel restart by successfully replaying this observation.
        // Compare with its requested cursor, not a snapshot advanced by a revoke.
        // A fresh revision also fences concurrent replies from the old projection.
        if (event.cursor < after) { this.refresh(); return }
        this.apply(event)
      }
    } catch (error) {
      if (!signal.aborted && revision === this.revision) {
        this.error = error instanceof Error ? error.message : "Access connection failed."
        this.publish()
        this.retry = setTimeout(() => { if (revision === this.revision) this.refresh() }, 2000)
      }
    }
  }
  async revoke(agentId: string | null): Promise<void> {
    this.sync()
    if (this.busy) return
    if (!this.snapshot || this.error) throw new Error(this.error ?? "Access grants are loading.")
    const revision = this.revision
    this.busy = true; this.publish()
    try { this.apply(await this.request({ op: "revoke_grants", agent_id: agentId }, revision, this.abort?.signal)) }
    catch (error) {
      if (revision === this.revision) { this.error = error instanceof Error ? error.message : "Revocation failed."; this.publish() }
      throw error
    } finally { if (revision === this.revision) { this.busy = false; this.publish() } }
  }
}
