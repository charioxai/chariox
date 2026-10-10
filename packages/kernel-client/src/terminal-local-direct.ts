// MP-08/MP-11 protocol 473: a carrier of the existing encrypted terminal
// protocol. Discovery permits attempts; relay-bound grants and key proofs admit.
import WebSocket from "ws"
import type { EncryptedRelayPayload, RelayTarget } from "./kernel-transport-frames.js"
import type { RelayClientIdentity } from "./relay-crypto.js"
import { buildRelayConnectFrame } from "./relay-transport.js"

type Grant = {
  endpoint: string; grant: string; kernel_id: string; endpoint_epoch: string
  paired_origin: string; expires_at_ms: number
}
type Lease = { socket: WebSocket; grant: Grant; sequence: number; key: string }
type LeaseDiagnostic = { cause: "lease_response_refused" | "lease_transport_failed";
  retrying?: boolean; sequenceMatches?: boolean; expired?: boolean }
const endpointPattern = /^ws:\/\/127\.0\.0\.1:([1-9]\d{0,4})\/v1\/browser$/

export class TerminalLocalDirect {
  private readonly leases = new Set<Lease>()
  private timer: NodeJS.Timeout | null = null
  private retry: NodeJS.Timeout | null = null
  private cooldownUntil = 0
  private closed = false
  private epoch = 0
  private readonly pending = new Set<WebSocket>()
  private readonly sockets = new WeakSet<WebSocket>()

  constructor(private readonly input: {
    // The client's current relay identity; the shared relay renewal keeps it fresh.
    relayUrl: string; token(): string; target: RelayTarget; identity: RelayClientIdentity
    eligible(): boolean; retryCarrier(): void
    onDiagnostic?(diagnostic: LeaseDiagnostic): void
  }) {}

  isLocal(socket: WebSocket | null): boolean { return socket !== null && this.sockets.has(socket) }

  async open(expectedKey: string): Promise<WebSocket | null> {
    if (this.closed || !this.input.eligible() || Date.now() < this.cooldownUntil) return null
    const epoch = this.epoch
    try {
      const [body] = await this.authorize([{ local_terminal_connect: {} }], expectedKey)
      const grant = body?.LocalTerminalConnectIssued as Grant | undefined
      if (!grant || !endpointPattern.test(grant.endpoint) || Number(new URL(grant.endpoint).port) > 65535
        || grant.kernel_id !== this.input.target.daemon_id || !grant.grant || !grant.endpoint_epoch
        || grant.expires_at_ms <= Date.now() || grant.expires_at_ms > Date.now() + 40_000) throw Error("invalid terminal direct grant")
      const origin = new URL(grant.paired_origin)
      if (origin.origin !== grant.paired_origin || (origin.protocol !== "https:"
        && !(origin.protocol === "http:" && ["localhost", "127.0.0.1"].includes(origin.hostname)))) throw Error("invalid paired terminal Origin")
      const socket = await this.prove(grant, expectedKey)
      if (this.closed || this.epoch !== epoch || !this.input.eligible()) { socket.terminate(); return null }
      const lease = { socket, grant, sequence: 1, key: expectedKey }
      this.leases.add(lease); this.sockets.add(socket)
      socket.once("close", () => {
        this.leases.delete(lease)
        if (!this.closed) this.cooldown()
        if (!this.leases.size) { clearTimeout(this.timer!); this.timer = null }
      })
      this.scheduleRenewal(10_000)
      return socket
    } catch { if (!this.closed) this.cooldown(); return null }
  }

  close(): void {
    this.closed = true; this.epoch++
    clearTimeout(this.timer!); clearTimeout(this.retry!)
    this.timer = null; this.retry = null
    for (const socket of this.pending) socket.terminate()
    for (const lease of this.leases) lease.socket.terminate()
    this.pending.clear(); this.leases.clear()
  }

  private cooldown(): void {
    if (Date.now() < this.cooldownUntil) return
    this.cooldownUntil = Date.now() + 30_000
    this.retry = setTimeout(() => {
      this.retry = null
      if (!this.closed && this.input.eligible()) this.input.retryCarrier()
    }, 30_000)
    this.retry.unref()
  }

  private scheduleRenewal(delay: number): void {
    if (this.closed || this.timer || !this.leases.size) return
    this.timer = setTimeout(() => { this.timer = null; void this.renew(false) }, delay)
    this.timer.unref()
  }

  private async renew(retrying: boolean): Promise<void> {
    const leases = [...this.leases].filter(lease => lease.socket.readyState === WebSocket.OPEN)
    if (!leases.length || this.closed) return
    try {
      // Every attempt spends a sequence, including a lost response. Renewal
      // always uses the real relay; a direct socket never extends its own lease.
      const attempts = leases.map(lease => lease.sequence++)
      const replies = await this.authorize(leases.map((lease, index) => ({ local_terminal_renew: {
        grant: lease.grant.grant, sequence: attempts[index],
      } })), leases[0]!.key)
      for (const [index, lease] of leases.entries()) {
        const reply = replies[index]?.LocalTerminalLeaseRenewed as { expires_at_ms?: number; next_sequence?: number } | undefined
        if (lease.key !== leases[0]!.key || reply?.next_sequence !== attempts[index]! + 1
          || !reply.expires_at_ms || reply.expires_at_ms <= Date.now()) {
          this.diagnose({ cause: "lease_response_refused", sequenceMatches: reply?.next_sequence === attempts[index]! + 1,
            expired: !reply?.expires_at_ms || reply.expires_at_ms <= Date.now() })
          lease.socket.terminate()
        }
      }
      this.scheduleRenewal(10_000)
    } catch {
      this.diagnose({ cause: "lease_transport_failed", retrying })
      if (!retrying && !this.closed) {
        this.timer = setTimeout(() => { this.timer = null; void this.renew(true) }, 1_000)
        this.timer.unref()
      } else for (const lease of leases) lease.socket.terminate()
    }
  }

  private diagnose(diagnostic: LeaseDiagnostic): void {
    try { this.input.onDiagnostic?.(diagnostic) } catch { /* Observation must not change lease handling. */ }
  }

  private authorize(bodies: readonly unknown[], expectedKey: string): Promise<Record<string, unknown>[]> {
    const { identity, target } = this.input
    const results = new Map<number, Record<string, unknown>>()
    return this.exchange(this.input.relayUrl, {}, (socket, frame, resolve, reject) => {
      if (frame.kind === "client_connected") {
        const connectedTarget = frame.target as RelayTarget | undefined
        if (frame.daemon_public_key !== expectedKey || connectedTarget?.daemon_id !== target.daemon_id) { reject(Error("terminal relay identity changed")); return }
        bodies.forEach((body, index) => socket.send(JSON.stringify({ kind: "client_request", request_id: String(index),
          target, encrypted_request: identity.encrypt(expectedKey, JSON.stringify(body)) })))
        return
      }
      if (frame.kind === "client_response") {
        const index = Number(frame.request_id)
        if (!Number.isSafeInteger(index) || index < 0 || index >= bodies.length || results.has(index)) { reject(Error("invalid terminal lease response")); return }
        if (frame.error || !frame.encrypted_response) { reject(Error("terminal lease refused")); return }
        results.set(index, JSON.parse(identity.decrypt(frame.encrypted_response as EncryptedRelayPayload, expectedKey)))
        if (results.size === bodies.length) resolve(bodies.map((_, index) => results.get(index)!))
        return
      }
      reject(Error("unexpected terminal relay frame"))
    }, socket => socket.send(JSON.stringify(buildRelayConnectFrame(this.input.token(), target))))
  }

  private prove(grant: Grant, expectedKey: string): Promise<WebSocket> {
    let challenge: string | null = null
    return this.exchange(grant.endpoint, { headers: { Origin: grant.paired_origin } }, (socket, frame, resolve, reject) => {
      if (frame.kind === "local_challenge" && challenge === null) {
        if (frame.kernel_id !== grant.kernel_id || frame.endpoint_epoch !== grant.endpoint_epoch || typeof frame.challenge !== "string") { reject(Error("terminal endpoint identity mismatch")); return }
        challenge = frame.challenge
        socket.send(JSON.stringify({ kind: "local_connect", proof: this.input.identity.encrypt(expectedKey, JSON.stringify({
          grant: grant.grant, challenge, origin: grant.paired_origin, kernel_id: grant.kernel_id, endpoint_epoch: grant.endpoint_epoch,
        })) }))
        return
      }
      if (frame.kind === "local_connected" && challenge !== null && frame.daemon_public_key === expectedKey) {
        const proof = JSON.parse(this.input.identity.decrypt(frame.proof as EncryptedRelayPayload, expectedKey))
        if (proof.grant !== grant.grant || proof.challenge !== challenge) { reject(Error("terminal kernel proof mismatch")); return }
        resolve(socket); return
      }
      reject(Error("terminal direct proof refused"))
    })
  }

  private exchange<T>(url: string, options: WebSocket.ClientOptions,
    frame: (socket: WebSocket, body: Record<string, unknown>, resolve: (result: T) => void, reject: (error: Error) => void) => void,
    onOpen?: (socket: WebSocket) => void): Promise<T> {
    if (this.closed) return Promise.reject(Error("terminal direct client closed"))
    return new Promise<T>((resolve, reject) => {
      const socket = new WebSocket(url, { ...options, handshakeTimeout: 5_000, maxPayload: 2 * 1024 * 1024 })
      this.pending.add(socket)
      const timer = setTimeout(() => finish(Error("terminal authorization timed out")), 5_000)
      const onMessage = (data: WebSocket.RawData) => {
        try { if (data.toString().length > 16 * 1024) throw Error("oversized authorization frame"); frame(socket, JSON.parse(String(data)), result => finish(null, result), error => finish(error)) }
        catch { finish(Error("invalid terminal authorization frame")) }
      }
      const onError = () => finish(Error("terminal authorization unavailable"))
      const onClose = () => finish(Error("terminal authorization closed"))
      let settled = false
      const finish = (error: Error | null, result?: T) => {
        if (settled) return
        settled = true; clearTimeout(timer); this.pending.delete(socket)
        socket.off("message", onMessage); socket.off("error", onError); socket.off("close", onClose)
        // Prevent a late error between retiring a failed upgrade and close.
        socket.on("error", () => {})
        if (error || result !== socket) socket.terminate()
        if (error) reject(error); else resolve(result!)
      }
      socket.on("message", onMessage); socket.once("error", onError); socket.once("close", onClose)
      if (onOpen) socket.once("open", () => { try { onOpen(socket) } catch { onError() } })
    })
  }
}
