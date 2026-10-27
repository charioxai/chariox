import WebSocket from "ws"
import { createHash } from "node:crypto"
import type { RelayConnectedFrame, RelayCloseFrame, RelayTarget } from "./kernel-transport-frames.js"
import { buildRelayConnectFrame } from "./relay-transport.js"
import { LocalIpcError } from "./local-ipc-error.js"

export const relayAuthorizationRenewalCapability = "terminal_relay_authorization_renewal_v1"
export const relayAuthorizationRenewalMinimumProtocolVersion = 472

export type RelayAuthorizationIssuer = { endpoint: string; daemonId: string }

// Issuer routes are public bootstrap metadata. Local authentication stays in
// the private profile; never send its bearer credential to a remote endpoint.
export function isLocalRelayIssuerEndpoint(endpoint: string): boolean {
  if (typeof endpoint !== "string") return false
  if (endpoint.startsWith("/")) return true
  if (endpoint.startsWith("ws+unix:///")) return !/[?#]/.test(endpoint)
  try {
    const url = new URL(endpoint)
    return url.protocol === "ws:" && ["127.0.0.1", "[::1]", "localhost"].includes(url.hostname)
      && !url.username && !url.password && !url.search && !url.hash && ["/", "/kernel"].includes(url.pathname)
  } catch { return false }
}

export function relayCloseError(reason: string): LocalIpcError {
  const temporary = reason === "target daemon disconnected from relay" || reason === "target daemon is not connected to relay"
  return new LocalIpcError("relay transport", reason, temporary ? "connection_closed" : "authorization_denied", temporary)
}

export type RelayAuthorization = {
  sub: string; subject_kind: string; realm_id: string; account_id?: string;
  user_id?: string; machine_id?: string; client_id?: string;
  session_id?: string; public_key_thumbprint: string; exp: number;
  allowed_actions: string[]; allowed_targets?: string[] | null
}

// Public scheduling/scope metadata only. The relay verifies the signature.
export function relayAuthorization(token: string | null): RelayAuthorization | null {
  if (!token || token.length > 16_384) return null
  const parts = token.split(".")
  if (parts.length !== 3) return null
  try {
    const claims = JSON.parse(Buffer.from(parts[1]!, "base64url").toString("utf8"))
    if (claims.subject_kind !== "client" || typeof claims.sub !== "string" || !claims.sub || typeof claims.realm_id !== "string" || !claims.realm_id || !Number.isFinite(claims.exp) || !/^[0-9a-f]{64}$/.test(claims.public_key_thumbprint) || !Array.isArray(claims.allowed_actions) || !claims.allowed_actions.every((action: unknown) => typeof action === "string") || claims.allowed_targets != null && (!Array.isArray(claims.allowed_targets) || !claims.allowed_targets.every((target: unknown) => typeof target === "string"))) return null
    return claims as RelayAuthorization
  } catch { return null }
}

export function requireRenewedRelayAuthorization(previous: RelayAuthorization, token: string, target: string): RelayAuthorization {
  const next = relayAuthorization(token)
  // A machine-only target cannot act as the issuer of an account-client grant.
  // Recognize the kernel's exact machine-subject mapping only to report that
  // limitation; never admit the replacement identity or extend the old grant.
  const hash = (value: string) => createHash("sha256").update(value).digest("hex")
  const machineMapped = previous.machine_id === undefined && previous.client_id === previous.sub
    && !previous.sub.startsWith("machine-client:") && typeof next?.machine_id === "string"
    && next.sub === `machine-client:${hash(next.machine_id)}:${hash(previous.sub)}` && next.client_id === next.sub
  const expected = machineMapped ? {...previous, sub: next!.sub, client_id: next!.client_id, machine_id: next!.machine_id} : previous
  const identity = ["sub", "subject_kind", "realm_id", "account_id", "user_id", "machine_id", "client_id", "session_id", "public_key_thumbprint"] as const
  if (!next || identity.some(key => next[key] !== expected[key]) || next.exp * 1000 <= Date.now() || next.exp <= previous.exp || next.allowed_actions.length !== previous.allowed_actions.length || previous.allowed_actions.some(action => !next.allowed_actions.includes(action)) || previous.allowed_targets != null && (next.allowed_targets == null || next.allowed_targets.some(value => !previous.allowed_targets!.includes(value))) || next.allowed_targets != null && !next.allowed_targets.includes(target)) {
    throw new LocalIpcError("renew relay authorization", "Relay authorization renewal returned an invalid identity, key or scope", "authorization_denied", false)
  }
  if (machineMapped) throw new LocalIpcError("renew relay authorization", `This machine-only target cannot renew an account-issued terminal grant. The current connection remains valid until ${new Date(previous.exp * 1000).toISOString()}. Obtain a fresh grant from the account-linked issuing kernel.`, "relay_renewal_authority_unavailable", false)
  return next
}

export class RelayAuthorizationRenewal {
  private timer?: ReturnType<typeof setTimeout>
  private expiryTimer?: ReturnType<typeof setTimeout>
  private stopped = false
  private authorityUnavailable: "machine" | "issuer" | null = null
  constructor(private expiresAtMs: number, private readonly renew: () => Promise<number>, private readonly refused: (message?: string) => void, private readonly notice: (message: string) => void = () => {}) { this.armExpiry(); this.schedule() }
  stop() { this.stopped = true; if (this.timer) clearTimeout(this.timer); if (this.expiryTimer) clearTimeout(this.expiryTimer) }
  private armExpiry() {
    if (this.expiryTimer) clearTimeout(this.expiryTimer)
    this.expiryTimer = setTimeout(() => { if (!this.stopped) this.expire() }, Math.max(0, this.expiresAtMs - Date.now()))
    this.expiryTimer.unref?.()
  }
  private schedule(retry = false) {
    if (this.stopped) return
    const remaining = this.expiresAtMs - Date.now()
    if (remaining <= 0) { this.expire(); return }
    const delay = retry ? Math.min(1_000, Math.max(25, remaining / 4)) : Math.max(0, remaining - Math.min(60_000, Math.max(1_000, remaining / 5)))
    this.timer = setTimeout(() => void this.run(), delay)
    this.timer.unref?.()
  }
  private async run() {
    try {
      const expiry = await this.renew()
      if (this.stopped) return
      this.authorityUnavailable = null
      this.expiresAtMs = expiry
      this.armExpiry()
      this.schedule()
    } catch (error) {
      if (this.stopped) return
      if (error instanceof LocalIpcError && error.code === "relay_renewal_authority_unavailable") {
        this.authorityUnavailable = "machine"
        this.notice(error.message) // Retain only the current admission's expiry timer.
      } else if (error instanceof LocalIpcError && error.code === "relay_renewal_issuer_unavailable") {
        if (!this.authorityUnavailable) this.notice(error.message)
        this.authorityUnavailable = "issuer"
        this.schedule(true)
      } else if (error instanceof LocalIpcError && !error.retryable) { this.refused(); this.stop() }
      else this.schedule(true)
    }
  }
  private expire() {
    this.refused(this.authorityUnavailable === "issuer" ? "The relay grant expired while the original issuing kernel was unavailable. Connection ended. Obtain a fresh grant from that issuing kernel." : this.authorityUnavailable === "machine" ? "The account-issued relay grant expired; this machine-only target cannot renew it. Connection ended. Obtain a fresh grant from the account-linked issuing kernel." : undefined)
    this.stop()
  }
}

// client_connect is already repeatable for the same bound target. Both lanes
// acknowledge the new grant without replacing their sockets or subscriptions.
export function reauthenticateRelaySocket(socket: WebSocket, token: string, target: RelayTarget, daemonKey: string, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const finish = (error?: Error) => {
      clearTimeout(timer)
      socket.off("message", acknowledge)
      socket.off("close", closed)
      signal.removeEventListener("abort", cancelled)
      error ? reject(error) : resolve()
    }
    const acknowledge = (data: WebSocket.RawData) => {
      let frame: RelayConnectedFrame | RelayCloseFrame
      try { frame = JSON.parse(String(data)) } catch { return }
      if (frame.kind === "close") finish(relayCloseError(frame.reason))
      if (frame.kind === "client_connected") {
        if ((frame.target?.daemon_id ?? null) !== (target.daemon_id ?? null) || (frame.target?.daemon_alias ?? null) !== (target.daemon_alias ?? null) || frame.daemon_public_key !== daemonKey) finish(new LocalIpcError("renew relay authorization", "Relay authorization renewal returned an invalid target or kernel key", "authorization_denied"))
        else finish()
      }
    }
    const closed = () => finish(new LocalIpcError("renew relay authorization", "Relay renewal connection closed", "connection_closed", true))
    const cancelled = () => finish(new LocalIpcError("renew relay authorization", "Renewal was cancelled", "client_closed"))
    const timer = setTimeout(() => finish(new LocalIpcError("renew relay authorization", "Relay renewal acknowledgement timed out", "connection_closed", true)), 10_000)
    socket.on("message", acknowledge)
    socket.once("close", closed)
    signal.addEventListener("abort", cancelled, {once:true})
    if (signal.aborted) { cancelled(); return }
    try { socket.send(JSON.stringify(buildRelayConnectFrame(token, target))) }
    catch { closed() }
  })
}
