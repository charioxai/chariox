import type { KernelBrowserCommand, UserDomainGrantEvent } from "./kernel-types-kernel-browser.js"

export const userDomainAccessMinimumProtocol = 494
// MP-08 / MP-10 / MP-11: only Room Computer grant consumers require 449.
export const roomComputerAccessMinimumProtocol = 494
// MP-08 / MP-11: bulk restore (`agent_id: null`) after "revoke all" requires 461.
export const roomComputerBulkGrantMinimumProtocol = 494
// LocalIpcClient has a 5s response-stall watchdog and no per-request options.
// Keep each owner observation below that watchdog on every client transport.
export const userDomainGrantPollWaitMs = 1000
export type UserDomainAccessClient = {
  readonly localDaemonProtocolVersion?: number
  send<T>(request: unknown, options?: { signal?: AbortSignal; timeoutMs?: number; retryOnTimeout?: boolean; beforeSend?: () => void }): Promise<T>
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
  constructor(private readonly deps: { client(): UserDomainAccessClient | null; bindingKey(): string }) {}
  subscribe(listener: () => void): () => void { this.listeners.add(listener); return () => this.listeners.delete(listener) }
  private publish(): void { for (const listener of this.listeners) listener() }
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
    if (!event || event.event !== "user_domain_grants_changed" || !Number.isSafeInteger(event.cursor) || event.cursor < 0) {
      throw new Error(response.Error?.message ?? "Invalid access snapshot from kernel.")
    }
    return event
  }
  private apply(event: UserDomainGrantEvent): void {
    if (this.snapshot && event.cursor < this.snapshot.cursor) return
    this.snapshot = event
    this.error = null; this.publish()
  }
  /** Call on connection/owner changes; starts one cursor feed even when the access view is closed. */
  sync(): void {
    const client = this.deps.client(), binding = this.deps.bindingKey()
    if (client === this.client && binding === this.binding && client?.localDaemonProtocolVersion === this.protocol) return
    this.stop()
    this.client = client; this.binding = binding; this.protocol = client?.localDaemonProtocolVersion
    if (!client || (client.localDaemonProtocolVersion ?? 0) < userDomainAccessMinimumProtocol) {
      this.error = "Update the connected kernel for Room Computer access (protocol 494)."; this.publish(); return
    }
    this.abort = new AbortController()
    void this.poll(this.revision, this.abort.signal)
  }
  stop(): void {
    ++this.revision; this.abort?.abort(); this.abort = null
    if (this.retry !== null) clearTimeout(this.retry)
    this.retry = null
    this.client = null; this.binding = ""; this.protocol = undefined; this.snapshot = null; this.busy = false; this.error = null
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
  async grantRoomComputer(agentId: string | null): Promise<void> {
    this.sync()
    const minimum = agentId === null ? roomComputerBulkGrantMinimumProtocol : roomComputerAccessMinimumProtocol
    if ((this.protocol ?? 0) < minimum) throw new Error(`Room Computer grants require kernel protocol ${minimum}.`)
    if (this.busy) return
    if (!this.snapshot || this.error) throw new Error(this.error ?? "Access grants are loading.")
    const revision = this.revision
    this.busy = true; this.publish()
    try { this.apply(await this.request({ op: "grant_room_computer", agent_id: agentId }, revision, this.abort?.signal)) }
    finally { if (revision === this.revision) { this.busy = false; this.publish() } }
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
