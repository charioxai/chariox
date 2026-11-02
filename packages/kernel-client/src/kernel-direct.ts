import { KernelDirectCarrier } from "./kernel-direct-carrier.js"
import { LocalDirectSecurityError } from "./kernel-direct-security.js"
import { LocalBrowserLeaseKeeper } from "./kernel-direct-lease.js"
import { createDataChannelOffer, dataChannelSupported, DataChannelSocket, directAttemptBudgetMs, relayStunServers, type DataChannelOffer } from "./kernel-data-channel.js"
import { parseFrame, type DirectSocketInput, type RelayCloseFrame, type RelayTarget, type EncryptedRelayPayload } from "./kernel-direct-types.js"
const kernelReloadWindowMs = 10_000

/*
 * MP-08/MP-11: direct carrier. The browser mints a single-use grant over its
 * authenticated relay lane, offers a WebRTC data channel to whichever kernel
 * address it reaches and proves the grant with the browser key bound into its
 * relay identity. After admission the channel speaks the ordinary encrypted
 * relay client frames. Any failure falls back to the relay for the same
 * kernel; a kernel-identity mismatch is surfaced and blocks further direct
 * attempts until the user retries.
 */

/**
 * Protocol 926: the kernel answers a WebRTC offer on its LAN, public and VPN
 * addresses and keeps the channel alive every second, so silence means a dead path.
 */
export const dataChannelMinimumProtocolVersion = 926

/** A 30-second lease token this young still leaves the next renewal ample room. */
const leaseTokenReuseMs = 5_000
const handshakeTimeoutMs = 5_000
const initialCooldownMs = 30_000
/** A kernel back from a hot reload within about a second: try it directly again at the reconnect pace. */
const kernelReloadCooldownMs = 250
/** A link that worked and dropped (kernel restart, network change) is retried soon: the kernel is most likely back within seconds. */
const lostLinkRetryMs = 2_000
const maximumCooldownMs = 10 * 60_000
const consentStoragePrefix = "chariox.kernel.local-direct."
const directOnlyStoragePrefix = "chariox.kernel.local-direct-only."

export type LocalDirectStatus = "relay" | "local" | "blocked"

/** The browser's choice of carrier for one kernel: Auto prefers the direct channel and falls back to the relay. */
export type CarrierMode = "auto" | "direct" | "relay"

export type LocalDirectGrant = {
  readonly pairedOrigin?: string
  readonly answer: string
  readonly grant: string
  readonly kernelId: string
  readonly endpointEpoch: string
  readonly daemonPublicKey: string
  readonly expiresAtMs: number
}

export type OpenLocalSocketInput = DirectSocketInput & {
  /** A background link can join an admitted carrier without creating or waiting for one. */
  readonly reuseOnly?: boolean
  readonly signal?: AbortSignal
  /** Keep automatic discovery from delaying the ordinary relay attachment. */
  readonly onDiscoveryPending?: () => void
  readonly relayUrls: readonly string[]
  readonly localDaemonProtocolVersion: number | undefined
}

type LocalLease<TIdentity> = {
  readonly socket: WebSocket
  readonly grant: LocalDirectGrant
  readonly input: OpenLocalSocketInput
  readonly keypair: TIdentity
  sequence: number
  expiresAtMs: number
}

type KernelLease<TIdentity> = {
  readonly keeper: LocalBrowserLeaseKeeper<LocalLease<TIdentity>>
  token: { readonly relayToken: Promise<string>; readonly requestedAtMs: number } | null
}

type KernelState = {
  cooldownUntilMs: number
  cooldownMs: number
  /** Until then the kernel is back from a hot reload any moment: failures wait only `kernelReloadCooldownMs`. */
  kernelReloadUntilMs: number
  /** A direct connection to this kernel has worked since this page loaded. */
  proven: boolean
  blockedReason: string | null
  lastStatus: LocalDirectStatus
  opening: number
}


export class KernelDirect<TIdentity> {
  private closed = false
  private readonly lifetime = new AbortController()
  private readonly preferences = new Map<string, boolean>()
  private readonly directOnlyPreferences = new Map<string, boolean>()
  private readonly kernels = new Map<string, KernelState>()
  private readonly leases = new Map<string, KernelLease<TIdentity>>()
  private readonly carriers = new Map<string, Map<TIdentity, Promise<KernelDirectCarrier | null>>>()
  private readonly listeners = new Set<() => void>()
  private readonly consentListeners = new Set<(kernelRef: string, enabled: boolean) => void>()

  constructor(private readonly deps: {
    readonly bindingKeypair: (targetRef?: string | null) => Promise<TIdentity>
    /** A 30-second Cloud relay token for local lease admission and renewal. */
    readonly requestLease?: (target: RelayTarget) => Promise<{ readonly relayToken: string }>
    readonly storage?: Pick<Storage, "getItem" | "setItem" | "removeItem"> | null
    readonly origin?: () => string
    readonly userAgent?: () => string
    readonly nowMs?: () => number
    readonly authorize: (input: OpenLocalSocketInput, identity: TIdentity, requests: () => readonly unknown[]) => Promise<{ readonly daemonPublicKey: string; readonly bodies: readonly (Record<string, unknown> | null)[] }>
    readonly encrypt: (identity: TIdentity, kernelKey: string, text: string) => Promise<EncryptedRelayPayload> | EncryptedRelayPayload
    readonly decrypt: (identity: TIdentity, payload: EncryptedRelayPayload) => Promise<string> | string
    readonly attach: (socket: WebSocket, key: string, input: DirectSocketInput) => void
    readonly clientKind?: "browser" | "terminal"
    readonly createDataChannelOffer?: (iceServers: readonly RTCIceServer[]) => Promise<DataChannelOffer>
    readonly setTimeout?: (callback: () => void, delayMs: number) => ReturnType<typeof setTimeout> | number
    readonly clearTimeout?: (handle: ReturnType<typeof setTimeout> | number) => void
  }) {}

  /** An explicit retry restores the automatic preference and clears a remembered block. */
  enable(kernelRef: string): void {
    this.preferences.set(kernelRef, true)
    this.storage()?.setItem(consentStoragePrefix + kernelRef, "granted")
    const state = this.kernels.get(kernelRef) ?? freshState()
    Object.assign(state, freshState(), { opening: state.opening })
    this.kernels.set(kernelRef, state)
    this.notifyConsent(kernelRef, true)
  }

  disable(kernelRef: string): void {
    this.preferences.set(kernelRef, false)
    this.storage()?.setItem(consentStoragePrefix + kernelRef, "disabled")
    this.notifyConsent(kernelRef, false)
  }

  /** MP-08/MP-11: every client of the kernel switches carrier, not only the clicked one. */
  onConsentChange(listener: (kernelRef: string, enabled: boolean) => void): () => void {
    this.consentListeners.add(listener)
    return () => this.consentListeners.delete(listener)
  }

  /** Developer tool: Auto is the default; Relay only is the existing opt-out; Direct only never falls back to the relay. */
  carrierMode(kernelRef: string): CarrierMode {
    if (!this.consented(kernelRef)) return "relay"
    return this.directOnly(kernelRef) ? "direct" : "auto"
  }

  setCarrierMode(kernelRef: string, mode: CarrierMode): void {
    if (mode === "relay") { this.setDirectOnly(kernelRef, false); this.disable(kernelRef); return }
    this.setDirectOnly(kernelRef, mode === "direct")
    this.enable(kernelRef)
  }

  directOnly(kernelRef: string): boolean {
    return this.directOnlyPreferences.get(kernelRef) ?? this.storage()?.getItem(directOnlyStoragePrefix + kernelRef) === "granted"
  }

  private setDirectOnly(kernelRef: string, directOnly: boolean): void {
    this.directOnlyPreferences.set(kernelRef, directOnly)
    if (directOnly) this.storage()?.setItem(directOnlyStoragePrefix + kernelRef, "granted")
    else this.storage()?.removeItem(directOnlyStoragePrefix + kernelRef)
  }

  consented(kernelRef: string): boolean {
    return this.preferences.get(kernelRef) ?? (this.storage()?.getItem(consentStoragePrefix + kernelRef) !== "disabled")
  }

  isOpening(kernelRef: string): boolean {
    return (this.kernels.get(kernelRef)?.opening ?? 0) > 0
  }

  retryDelayMs(kernelRef: string): number {
    return Math.max(0, (this.kernels.get(kernelRef)?.cooldownUntilMs ?? 0) - this.nowMs())
  }

  status(kernelRef: string): LocalDirectStatus {
    return this.kernels.get(kernelRef)?.lastStatus ?? "relay"
  }

  blockedReason(kernelRef: string): string | null {
    return this.kernels.get(kernelRef)?.blockedReason ?? null
  }

  notifyCarrierChange(): void { this.notify() }

  onChange(listener: () => void): () => void {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  close(): void {
    this.closed = true
    this.lifetime.abort()
    this.carriers.clear()
    for (const lease of this.leases.values()) lease.keeper.close()
  }

  /** Returns an admitted direct socket, or null when the lane must use the relay. */
  async open(input: OpenLocalSocketInput): Promise<WebSocket | null> {
    const kernelRef = input.target.daemon_id ?? input.target.daemon_alias
    if (this.closed || !kernelRef || (input.localDaemonProtocolVersion ?? 0) < dataChannelMinimumProtocolVersion
      || !this.consented(kernelRef) || !this.deps.requestLease
      || !(this.deps.createDataChannelOffer || dataChannelSupported())) return null
    return input.reuseOnly ? this.openExistingCarrier(input, kernelRef) : this.openDataChannel(input, kernelRef)
  }

  private async openExistingCarrier(input: OpenLocalSocketInput, kernelRef: string): Promise<WebSocket | null> {
    const state = this.kernels.get(kernelRef)
    if (!state || state.blockedReason || state.cooldownUntilMs > this.nowMs()) return null
    const target = JSON.stringify([input.target.daemon_id ?? null, input.target.daemon_alias ?? null])
    const carriers = this.carriers.get(target)
    if (!carriers) return null
    const keypair = await this.deps.bindingKeypair(kernelRef)
    const pending = carriers.get(keypair)
    if (!pending) return null
    // A background link joins a proven channel without waiting for discovery.
    const carrier = await Promise.race([pending, Promise.resolve(null)])
    if (!carrier?.open || this.closed || input.signal?.aborted) return null
    const lane = carrier.openLane(input.signal)
    state.proven = true
    this.setStatus(state, "local")
    try { this.deps.attach(lane, carrier.daemonPublicKey, input) }
    catch (error) { lane.close(); throw error }
    return lane
  }

  private lease(kernelRef: string): KernelLease<TIdentity> {
    let lease = this.leases.get(kernelRef)
    if (!lease) {
      lease = {
        token: null,
        keeper: new LocalBrowserLeaseKeeper<LocalLease<TIdentity>>({
          renew: (leases) => this.renewLeases(kernelRef, leases),
          consented: () => this.consented(kernelRef),
          nowMs: () => this.nowMs(),
          onFailure: () => this.cooldown(this.kernels.get(kernelRef) ?? freshState()),
          ...(this.deps.setTimeout ? { setTimeout: this.deps.setTimeout } : {}),
          ...(this.deps.clearTimeout ? { clearTimeout: this.deps.clearTimeout } : {}),
        }),
      }
      this.leases.set(kernelRef, lease)
    }
    return lease
  }

  /** One Cloud lease token serves every lane, client and renewal of a kernel. */
  private leaseToken(kernelRef: string, target: RelayTarget): Promise<string> {
    const lease = this.lease(kernelRef)
    if (!lease.token || lease.token.requestedAtMs + leaseTokenReuseMs < this.nowMs()) {
      const token = { relayToken: this.deps.requestLease!(target).then((reply) => reply.relayToken), requestedAtMs: this.nowMs() }
      token.relayToken.catch(() => { if (lease.token === token) lease.token = null })
      lease.token = token
    }
    return lease.token.relayToken
  }

  /** Renews a kernel's leases over one relay lane; returns those the kernel refused. */
  private async renewLeases(kernelRef: string, leases: readonly LocalLease<TIdentity>[]): Promise<readonly LocalLease<TIdentity>[]> {
    const { input, keypair } = leases[0]!
    const relayToken = await this.leaseToken(kernelRef, input.target)
    // Every attempt, including each relay failover, spends its sequence: a
    // retry after a lost response must not look like a replay whether or not
    // the kernel applied it.
    let attempts: number[] = []
    const reply = await this.deps.authorize({ ...input, relayToken }, keypair, () => {
      attempts = leases.map((lease) => lease.sequence++)
      return leases.map((lease, index) => ({ [this.deps.clientKind === "terminal" ? "local_terminal_renew" : "local_browser_renew"]: { grant: lease.grant.grant, sequence: attempts[index] } }))
    })
    return leases.filter((lease, index) => {
      const renewed = reply.bodies[index]?.[this.deps.clientKind === "terminal" ? "LocalTerminalLeaseRenewed" : "LocalBrowserLeaseRenewed"] as { next_sequence?: unknown; expires_at_ms?: unknown } | undefined
      const refused = reply.daemonPublicKey !== lease.grant.daemonPublicKey || renewed?.next_sequence !== attempts[index]! + 1
        || typeof renewed.expires_at_ms !== "number" || !Number.isFinite(renewed.expires_at_ms) || renewed.expires_at_ms <= this.nowMs()
      if (!refused) lease.expiresAtMs = renewed!.expires_at_ms as number
      return refused
    })
  }

  /**
   * MP-08/MP-11 protocol 917: the same grant and handshake over a WebRTC data
   * channel to whichever kernel address the browser reaches (LAN, public,
   * Tailscale). Any failure falls back to the relay and retries after the
   * cooldown.
   */
  private async openDataChannel(input: OpenLocalSocketInput, kernelRef: string): Promise<WebSocket | null> {
    const state = this.kernels.get(kernelRef) ?? freshState()
    this.kernels.set(kernelRef, state)
    if (state.blockedReason || state.cooldownUntilMs > this.nowMs()) return null
    state.opening++
    const startedAtMs = this.nowMs()
    const callerSignal = input.signal
    const budget = new AbortController()
    input = { ...input, signal: AbortSignal.any([...(callerSignal ? [callerSignal] : []), budget.signal, this.lifetime.signal]) }
    let offer: DataChannelOffer | undefined
    let publish: ((carrier: KernelDirectCarrier | null) => void) | undefined
    let forget: (() => void) | undefined
    // The lease, ICE gathering and grant all precede the channel's own connect
    // timer; the relay must not wait for them past the budget.
    const yieldToRelay = (this.deps.setTimeout ?? setTimeout)(() => { input.onDiscoveryPending?.(); budget.abort(new Error("direct discovery timed out")) }, directAttemptBudgetMs)
    try {
      this.notify()
      input.signal?.throwIfAborted()
      const keypair = await abortable(this.deps.bindingKeypair(kernelRef), input.signal!)
      const carrierTarget = JSON.stringify([input.target.daemon_id ?? null, input.target.daemon_alias ?? null])
      let carriers = this.carriers.get(carrierTarget)
      if (!carriers) { carriers = new Map(); this.carriers.set(carrierTarget, carriers) }
      let existing: Promise<KernelDirectCarrier | null> | undefined
      while ((existing = carriers.get(keypair))) {
        const carrier = await abortable(existing, input.signal!)
        if (carrier?.open) {
          const lane = carrier.openLane(input.signal)
          state.proven = true
          this.setStatus(state, "local")
          this.deps.attach(lane, carrier.daemonPublicKey, input)
          return lane
        }
        if (carriers.get(keypair) === existing) carriers.delete(keypair)
      }
      const pending = new Promise<KernelDirectCarrier | null>(resolve => { publish = resolve })
      carriers.set(keypair, pending)
      const held = carriers
      forget = () => { if (held.get(keypair) === pending) held.delete(keypair) }

      const relayToken = await abortable(this.leaseToken(kernelRef, input.target), input.signal!)
      const preparing = (this.deps.createDataChannelOffer ?? createDataChannelOffer)(relayStunServers(input.relayUrls)).then(ready => {
        if (input.signal!.aborted) { ready.peer.close(); input.signal!.throwIfAborted() }
        return ready
      })
      offer = await abortable(preparing, input.signal!)
      const grant = await abortable(this.mintGrant({ ...input, relayToken }, keypair, offer.sdp), input.signal!)
      input.signal?.throwIfAborted()
      if (this.closed || !this.consented(kernelRef)) { offer.peer.close(); return null }
      await abortable(offer.peer.setRemoteDescription({ type: "answer", sdp: grant.answer }), input.signal!)
      const socket = await this.adoptCarrier(input, keypair, grant, new DataChannelSocket(offer.peer, offer.channel) as unknown as WebSocket)
      if (this.closed || input.signal?.aborted) { socket.close(); return null }
      // A sibling lane may outlive the caller that opened this carrier.
      this.lease(kernelRef).keeper.add({ socket, grant, input: { ...input, signal: this.lifetime.signal }, keypair, sequence: 1, expiresAtMs: grant.expiresAtMs })
      state.cooldownMs = 0
      state.cooldownUntilMs = 0
      state.proven = true
      this.setStatus(state, "local")
      input.logTransport("info", "direct_datachannel_connected", {
        lane: input.lane,
        elapsedMs: this.nowMs() - startedAtMs,
        target: input.targetForLogs,
      })
      const carrier = new KernelDirectCarrier(socket, grant.daemonPublicKey, close => {
        if (close?.kernel_reload === true) state.kernelReloadUntilMs = this.nowMs() + kernelReloadWindowMs
        else this.cooldown(state)
      })
      const lane = carrier.openLane(input.signal)
      this.deps.attach(lane, grant.daemonPublicKey, input)
      publish!(carrier)
      publish = undefined
      return lane
    } catch (error) {
      offer?.peer.close()
      if (callerSignal?.aborted) return null
      const message = error instanceof Error ? error.message : String(error)
      if (error instanceof LocalDirectSecurityError) {
        state.blockedReason = message
        this.setStatus(state, "blocked")
      } else {
        this.cooldown(state)
      }
      input.logTransport("debug", "direct_datachannel_fallback_to_relay", {
        lane: input.lane,
        message,
        elapsedMs: this.nowMs() - startedAtMs,
        blocked: state.blockedReason !== null,
        cooldownMs: state.cooldownMs,
        target: input.targetForLogs,
      })
      return null
    } finally {
      if (publish) { forget?.(); publish(null) }
      (this.deps.clearTimeout ?? clearTimeout)(yieldToRelay)
      state.opening--
      this.notify()
    }
  }

  /** Mints the grant over a short-lived relay lane authenticated like any other. */
  private async mintGrant(input: OpenLocalSocketInput, keypair: TIdentity, offer: string): Promise<LocalDirectGrant> {
    const { daemonPublicKey, bodies: [body] } = await this.deps.authorize(input, keypair,
      () => [{ [this.deps.clientKind === "terminal" ? "local_terminal_connect" : "local_browser_connect"]: { offer } }])
    if (!body) throw new Error("relay refused local authorization")
    const grant = body[this.deps.clientKind === "terminal" ? "LocalTerminalConnectIssued" : "LocalBrowserConnectIssued"] as {
      answer?: unknown; grant?: unknown; kernel_id?: unknown; endpoint_epoch?: unknown; expires_at_ms?: unknown; paired_origin?: string
    } | undefined
    if (typeof grant?.answer !== "string" || grant.answer.length === 0 || typeof grant.grant !== "string" || typeof grant.kernel_id !== "string"
      || typeof grant.endpoint_epoch !== "string"
      || typeof grant.expires_at_ms !== "number" || !Number.isFinite(grant.expires_at_ms) || grant.expires_at_ms <= 0) {
      throw new LocalDirectSecurityError("kernel returned an invalid local endpoint")
    }
    if (input.target.daemon_id && grant.kernel_id !== input.target.daemon_id) {
      throw new LocalDirectSecurityError("local connection grant names another kernel")
    }
    return { ...(grant.paired_origin ? { pairedOrigin: grant.paired_origin } : {}), answer: grant.answer, grant: grant.grant, kernelId: grant.kernel_id,
      endpointEpoch: grant.endpoint_epoch, daemonPublicKey, expiresAtMs: grant.expires_at_ms }
  }

  /** Proves the kernel behind the data channel with the grant's handshake, then attaches it as an ordinary relay socket. */
  private adoptCarrier(
    input: OpenLocalSocketInput,
    keypair: TIdentity,
    grant: LocalDirectGrant,
    socket: WebSocket,
  ): Promise<WebSocket> {
    return new Promise<WebSocket>((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout> | undefined
      // The handshake is bounded from the moment the channel is open.
      const onOpen = () => {
        timer ??= setTimeout(() => fail(new Error("local kernel handshake timed out")), handshakeTimeoutMs)
      }
      let settled = false
      let challenge: string | null = null
      const fail = (error: Error) => {
        if (settled) return
        settled = true
        cleanup()
        if (socket.readyState === 0 || socket.readyState === 1) {
          socket.close()
        }
        reject(error)
      }
      const onMessage = (event: MessageEvent) => {
        onOpen()
        if (settled) return
        void Promise.resolve().then(() => {
          if (settled) return null
          input.signal?.throwIfAborted()
          return this.handleHandshakeFrame(event.data, keypair, grant, challenge)
        }).then((next) => {
          if (settled || !next) return
          if (next.kind === "challenge") {
            challenge = next.challenge
            socket.send(next.reply)
            return
          }
          settled = true
          cleanup()
          resolve(socket)
        }).catch((error: unknown) => fail(error instanceof Error ? error : new Error(String(error))))
      }
      const onError = () => fail(new Error("local kernel endpoint is unreachable"))
      const onClose = (event: CloseEvent) => fail(new Error(event.reason || `local kernel endpoint closed (${event.code})`))
      const onAbort = () => fail(new DOMException("local attempt cancelled", "AbortError"))
      const cleanup = () => {
        clearTimeout(timer)
        input.signal?.removeEventListener("abort", onAbort)
        socket.removeEventListener("open", onOpen)
        socket.removeEventListener("message", onMessage)
        socket.removeEventListener("error", onError)
        socket.removeEventListener("close", onClose)
      }
      input.signal?.addEventListener("abort", onAbort, { once: true })
      if (input.signal?.aborted) { onAbort(); return }
      socket.addEventListener("open", onOpen)
      socket.addEventListener("message", onMessage)
      socket.addEventListener("error", onError)
      socket.addEventListener("close", onClose)
    })
  }

  private async handleHandshakeFrame(
    data: unknown,
    keypair: TIdentity,
    grant: LocalDirectGrant,
    challenge: string | null,
  ): Promise<{ readonly kind: "challenge"; readonly challenge: string; readonly reply: string } | { readonly kind: "connected" }> {
    const frame = parseFrame<LocalHandshakeFrame>(data)
    if (frame?.kind === "close") {
      throw new Error(`local kernel refused the connection: ${frame.reason}`)
    }
    if (frame?.kind === "local_challenge" && challenge === null) {
      if (frame.kernel_id !== grant.kernelId || frame.endpoint_epoch !== grant.endpointEpoch) {
        throw new LocalDirectSecurityError("local endpoint is not the kernel that issued the grant")
      }
      const proof = await this.deps.encrypt(keypair, grant.daemonPublicKey, JSON.stringify({
        grant: grant.grant,
        challenge: frame.challenge,
        origin: grant.pairedOrigin ?? this.origin(),
        kernel_id: grant.kernelId,
        endpoint_epoch: grant.endpointEpoch,
      }))
      return {
        kind: "challenge",
        challenge: frame.challenge,
        reply: JSON.stringify({ kind: "local_connect", proof }),
      }
    }
    if (frame?.kind === "local_connected" && challenge !== null) {
      // Only the holder of the pinned kernel key could read the grant and
      // answer with it; an impostor listener on the port cannot.
      if (frame.daemon_public_key !== grant.daemonPublicKey || frame.proof?.sender_public_key !== grant.daemonPublicKey) {
        throw new LocalDirectSecurityError("local endpoint did not prove the pinned kernel key")
      }
      let confirmed: { readonly grant?: unknown; readonly challenge?: unknown }
      try {
        confirmed = JSON.parse(await this.deps.decrypt(keypair, frame.proof))
      } catch {
        throw new LocalDirectSecurityError("local endpoint did not prove the pinned kernel key")
      }
      if (confirmed.grant !== grant.grant || confirmed.challenge !== challenge) {
        throw new LocalDirectSecurityError("local endpoint proof does not match this connection")
      }
      return { kind: "connected" }
    }
    throw new Error("local kernel sent an unexpected handshake frame")
  }

  /** Every failed attempt doubles the wait up to the ceiling; a success, a returning kernel or a network change restarts it, from a short wait once the link has worked. */
  private cooldown(state: KernelState): void {
    if (state.cooldownUntilMs > this.nowMs()) { this.setStatus(state, "relay"); return }
    if (this.nowMs() < state.kernelReloadUntilMs) {
      state.cooldownUntilMs = this.nowMs() + kernelReloadCooldownMs
      this.setStatus(state, "relay")
      return
    }
    state.cooldownMs = Math.min(Math.max(state.cooldownMs * 2, state.proven ? lostLinkRetryMs : initialCooldownMs), maximumCooldownMs)
    state.cooldownUntilMs = this.nowMs() + state.cooldownMs
    this.setStatus(state, "relay")
    this.notify()
  }

  /** The kernel came back or the network changed: what failed before may work now, so the kernel (every kernel when none is named) retries at once. */
  resetCooldowns(only?: string): void {
    for (const [kernelRef, state] of this.kernels) {
      if ((only !== undefined && kernelRef !== only) || state.blockedReason || !this.consented(kernelRef)) continue
      state.cooldownUntilMs = 0
      state.cooldownMs = 0
    }
    this.notify()
  }

  private setStatus(state: KernelState, status: LocalDirectStatus): void {
    if (state.lastStatus !== status) {
      state.lastStatus = status
      this.notify()
    }
  }

  private notify(): void {
    for (const listener of this.listeners) listener()
  }

  private notifyConsent(kernelRef: string, enabled: boolean): void {
    for (const listener of [...this.consentListeners]) listener(kernelRef, enabled)
    this.notify()
  }

  private origin(): string {
    return this.deps.origin?.() ?? globalThis.location.origin
  }

  private nowMs(): number {
    return this.deps.nowMs?.() ?? Date.now()
  }

  private storage(): Pick<Storage, "getItem" | "setItem" | "removeItem"> | null {
    try {
      const storage = this.deps.storage === undefined ? globalThis.localStorage : this.deps.storage
      if (!storage) return null
      // Profile storage can be disabled or full. It must not interrupt admission.
      return {
        getItem: (key) => { try { return storage.getItem(key) } catch { return null } },
        setItem: (key, value) => { try { storage.setItem(key, value) } catch { /* best effort */ } },
        removeItem: (key) => { try { storage.removeItem(key) } catch { /* best effort */ } },
      }
    } catch { return null }
  }
}

type LocalHandshakeFrame =
  | RelayCloseFrame
  | {
    readonly kind: "local_challenge"
    readonly kernel_id: string
    readonly endpoint_epoch: string
    readonly challenge: string
  }
  | {
    readonly kind: "local_connected"
    readonly daemon_public_key: string
    readonly proof: EncryptedRelayPayload | null
  }

function freshState(): KernelState {
  return { cooldownUntilMs: 0, cooldownMs: 0, kernelReloadUntilMs: 0, proven: false, blockedReason: null, lastStatus: "relay", opening: 0 }
}

/** A slow discovery stage must give the ordinary relay lane back its budget. */
function abortable<T>(pending: Promise<T>, signal: AbortSignal): Promise<T> {
  return new Promise((resolve, reject) => {
    const abort = () => { signal.removeEventListener("abort", abort); reject(signal.reason) }
    if (signal.aborted) { void pending.catch(() => {}); abort(); return }
    signal.addEventListener("abort", abort, { once: true })
    pending.then(value => { signal.removeEventListener("abort", abort); resolve(value) }, error => {
      signal.removeEventListener("abort", abort); reject(error)
    })
  })
}
