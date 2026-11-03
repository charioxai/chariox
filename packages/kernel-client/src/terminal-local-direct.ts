// MP-08/MP-10/MP-11: environment adapters around the shared Web/TUI carrier.
import WebSocket from "ws"
import { EventEmitter } from "node:events"
import { KernelDirect, dataChannelMinimumProtocolVersion } from "./kernel-direct.js"
import { createDataChannelOffer, type DataChannelSocket } from "./kernel-data-channel.js"
import { TerminalDirectAuthorization } from "./terminal-direct-authorization.js"
import type { RelayTarget } from "./kernel-transport-frames.js"
import type { RelayClientIdentity } from "./relay-crypto.js"

/** Presents the shared browser-shaped carrier to the Node IPC lane. */
class NodeDirectSocket extends EventEmitter {
  constructor(private readonly socket: DataChannelSocket) {
    super()
    socket.addEventListener("message", event => this.emit("message", Buffer.from((event as MessageEvent<string>).data)))
    socket.addEventListener("pong", () => this.emit("pong"))
    socket.addEventListener("close", event => {
      const close = event as CloseEvent
      this.emit("close", close.code, Buffer.from(close.reason))
    }, { once: true })
    // The close event owns recovery, just as a failed ws carrier does.
    socket.addEventListener("error", () => {})
  }
  get readyState(): number { return this.socket.readyState }
  get bufferedAmount(): number { return this.socket.bufferedAmount }
  send(data: string): void { this.socket.send(data) }
  ping(): void { this.socket.ping() }
  close(): void { this.socket.close() }
  terminate(): void { this.socket.close(1006, "direct carrier retired") }
}

export class TerminalLocalDirect {
  private readonly direct: KernelDirect<RelayClientIdentity>
  private readonly authorization: TerminalDirectAuthorization
  private readonly sockets = new WeakSet<WebSocket>()
  private retry: NodeJS.Timeout | undefined
  private closed = false

  constructor(private readonly input: {
    relayUrl: string; token(): string; target: RelayTarget; identity: RelayClientIdentity
    eligible(): boolean; retryCarrier(): void
    onDiagnostic?(diagnostic: { cause: "direct_fallback" }): void
  }) {
    this.authorization = new TerminalDirectAuthorization(input)
    this.direct = new KernelDirect({
      clientKind: "terminal", storage: null,
      bindingKeypair: async () => input.identity,
      requestLease: async () => ({ relayToken: input.token() }),
      authorize: async (request, _identity, bodies) => ({ daemonPublicKey: this.expectedKey!, bodies: await this.authorization.authorize(bodies(), this.expectedKey!, request.signal) }),
      encrypt: (identity, key, text) => identity.encrypt(key, text),
      decrypt: (identity, payload) => identity.decrypt(payload, this.expectedKey!),
      createDataChannelOffer: async servers => {
        const { RTCPeerConnection } = await import("werift")
        return createDataChannelOffer(servers, RTCPeerConnection as unknown as typeof globalThis.RTCPeerConnection)
      },
      attach: (socket, key, request) => {
        request.onConnected(socket, key); request.onSettled()
        socket.addEventListener("close", () => request.onClosed(socket, "direct carrier closed", {}), { once: true })
      },
    })
    this.direct.onChange(() => {
      const ref = input.target.daemon_id ?? input.target.daemon_alias
      if (!ref || this.closed || this.direct.isOpening(ref) || this.direct.status(ref) !== "relay" || this.retry || this.direct.blockedReason(ref)) return
      this.retry = setTimeout(() => {
        this.retry = undefined
        if (!this.closed && input.eligible()) input.retryCarrier()
      }, Math.max(1, this.direct.retryDelayMs(ref)))
      this.retry.unref()
    })
  }
  private expectedKey: string | undefined
  isLocal(socket: WebSocket | null): boolean { return socket !== null && this.sockets.has(socket) }
  async open(expectedKey: string): Promise<WebSocket | null> {
    if (this.closed || !this.input.eligible()) return null
    // The identity is pinned by the admitted relay lane before discovery begins.
    if (this.expectedKey && this.expectedKey !== expectedKey) return null
    this.expectedKey = expectedKey
    const socket = await this.direct.open({
      relayUrls: [this.input.relayUrl], relayToken: this.input.token(), target: this.input.target,
      localDaemonProtocolVersion: dataChannelMinimumProtocolVersion, lane: "control", attempt: 0, targetForLogs: this.input.target,
      logTransport: (_level, event) => { if (event === "direct_datachannel_fallback_to_relay") this.input.onDiagnostic?.({ cause: "direct_fallback" }) }, onSettled: () => {}, onConnected: () => {}, onMessage: () => {}, onClosed: () => {},
    })
    if (!socket) return null
    if (this.closed) { socket.close(); return null }
    const node = new NodeDirectSocket(socket as unknown as DataChannelSocket) as unknown as WebSocket
    this.sockets.add(node)
    return node
  }
  close(): void {
    this.closed = true
    clearTimeout(this.retry)
    this.direct.close(); this.authorization.close()
  }
}
