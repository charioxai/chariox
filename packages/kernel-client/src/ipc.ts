import net from "node:net"
import {
  createCipheriv,
  createDecipheriv,
  createECDH,
  hkdfSync,
  randomBytes,
  randomUUID,
} from "node:crypto"

import WebSocket from "ws"

const IPC_TIMEOUT_MS = 120_000
const DEFAULT_KERNEL_EVENT_STALE_MS = 0
const DEFAULT_KERNEL_PING_INTERVAL_MS = 5_000
const DEFAULT_KERNEL_MAX_MISSED_PONGS = 2
const IPC_WEBSOCKET_CLOSE_TIMEOUT_MS = 1_000

type IpcEnvelope<TResponse> = {
  response: TResponse | null
  error: string | null
}

type KernelTransportResponseFrame<TResponse> = {
  type: "response"
  request_id: string
  response: TResponse | null
  error: KernelTransportError | null
}

type KernelTransportEventFrame<TEvent> = {
  type: "event"
  event_id: number
  event: TEvent
}

type KernelTransportError = {
  code: string
  message: string
  retryable: boolean
}

type RelayTarget = {
  daemon_id?: string | null
  daemon_alias?: string | null
}

type RelayConnectFrame = {
  kind: "client_connect"
  auth_token: string
  target: RelayTarget
}

type RelayConnectedFrame = {
  kind: "client_connected"
  target: RelayTarget
  daemon_public_key: string
}

type EncryptedRelayPayload = {
  sender_public_key: string
  nonce: string
  ciphertext: string
}

type RelayRequestFrame = {
  kind: "client_request"
  request_id: string
  target: RelayTarget
  encrypted_request: EncryptedRelayPayload
}

type RelayResponseFrame<TResponse> = {
  kind: "client_response"
  request_id: string
  encrypted_response: EncryptedRelayPayload | null
  error: KernelTransportError | null
}

type RelaySubscribeFrame = {
  kind: "client_subscribe"
  request_id: string
  subscription_id: string
  target: RelayTarget
  session_id: string
  attachment_id: string
  client_public_key: string
  subscription_scope?: string
  resume_from_event_id: number | null
}

type RelayUnsubscribeFrame = {
  kind: "client_unsubscribe"
  request_id: string
  subscription_id: string
  client_public_key: string
}

type RelayEventFrame = {
  kind: "client_event"
  subscription_id: string
  event_id: number
  encrypted_event: EncryptedRelayPayload
}

type RelayCloseFrame = {
  kind: "close"
  reason: string
}

type PendingRequest<TResponse> = {
  resolve: (value: TResponse) => void
  reject: (error: LocalIpcError) => void
  timeout: NodeJS.Timeout
  relayPrivateKey: Buffer | null
  lane: KernelSocketLane
}

type KernelSocketLane = "control" | "event"

export type KernelEvent =
  | {
    event: "terminal_output"
    records: Array<Record<string, unknown>>
  }
  | {
    event: "runtime_notices"
    notices: Array<Record<string, unknown>>
  }
  | {
    event: "assistant_message_completed"
    session_id: string
    provider_run_id: string
    agent_id: string | null
    message_id: string
    completed_at_ms: number
  }
  | {
    event: "session_snapshot"
    session: Record<string, unknown>
    provider_run: Record<string, unknown> | null
    agent_activity: Record<string, unknown>
  }
  | {
    event: "session_unavailable"
    session_id: string
    message: string
  }
  | {
    event: "relay_status_changed"
    status: {
      configured: boolean
      connected: boolean
      relay_url?: string | null
      relay_token_configured: boolean
      daemon_id: string
      machine_id: string
      machine_alias?: string | null
    }
  }
  | {
    event: "remote_machines_changed"
    machines: Array<{
      machine_id: string
      machine_alias?: string | null
      registry_alias?: string | null
      display_name: string
      trust_status: "approved" | "pending" | "forgotten"
      online: boolean
      pending: boolean
      kernel_count: number
      available_providers?: string[]
    }>
  }
  | {
    event: "waiting_room_inventory_changed"
    inventory_version: string
  }
  | {
    event: "heartbeat"
    session_id: string
  }
  | {
    event: "transport_resumed"
    session_id: string
    resumed_from_event_id: number | null
  }
  | {
    event: "replay_gap"
    session_id: string
    requested_from_event_id: number
    first_retained_event_id: number | null
    latest_event_id: number | null
    message: string
  }
  | {
    event: "transport_closed"
    message: string
  }

export class LocalIpcError extends Error {
  constructor(
    readonly operation: string,
    message: string,
    readonly code: string | null = null,
    readonly retryable = false,
  ) {
    super(`kernel transport \`${operation}\` failed: ${message}`)
    this.name = "LocalIpcError"
  }
}

type KernelSubscriptionState = {
  sessionId: string
  attachmentId: string
  scope: "session" | "waiting_room_inventory"
  relaySubscriptionId: string | null
  relayPrivateKey: Buffer | null
}

type LocalIpcClientOptions = {
  relayAuthToken?: string | undefined
  targetDaemonId?: string | undefined
  targetDaemonAlias?: string | undefined
  kernelEventStaleMs?: number | undefined
  kernelPingIntervalMs?: number | undefined
  kernelMaxMissedPongs?: number | undefined
}

export class LocalIpcClient {
  readonly socketPath: string
  private readonly relayAuthToken: string | null
  private readonly relayTarget: RelayTarget | null
  private controlWebsocket: WebSocket | null = null
  private eventWebsocket: WebSocket | null = null
  private controlWebsocketConnectPromise: Promise<WebSocket> | null = null
  private eventWebsocketConnectPromise: Promise<WebSocket> | null = null
  private pending = new Map<string, PendingRequest<unknown>>()
  private eventHandlers = new Set<(event: KernelEvent) => void>()
  private activeKernelSubscription: KernelSubscriptionState | null = null
  private reconnectTimeout: NodeJS.Timeout | null = null
  private reconnectDelayMs = 250
  private lastReceivedEventId: number | null = null
  private lastKernelEventAtMs = 0
  private kernelEventWatchdog: NodeJS.Timeout | null = null
  private controlHeartbeat: NodeJS.Timeout | null = null
  private eventHeartbeat: NodeJS.Timeout | null = null
  private missedControlPongs = 0
  private missedEventPongs = 0
  private suppressNextControlCloseEvent = false
  private suppressNextEventCloseEvent = false
  private controlRelayDaemonPublicKey: string | null = null
  private eventRelayDaemonPublicKey: string | null = null
  private readonly kernelEventStaleMs: number
  private readonly kernelPingIntervalMs: number
  private readonly kernelMaxMissedPongs: number

  constructor(endpoint: string, options: LocalIpcClientOptions = {}) {
    this.socketPath = endpoint
    const staleMs = options.kernelEventStaleMs ?? DEFAULT_KERNEL_EVENT_STALE_MS
    this.kernelEventStaleMs = staleMs > 0 ? Math.max(staleMs, 250) : 0
    this.kernelPingIntervalMs = Math.max(options.kernelPingIntervalMs ?? DEFAULT_KERNEL_PING_INTERVAL_MS, 250)
    this.kernelMaxMissedPongs = Math.max(options.kernelMaxMissedPongs ?? DEFAULT_KERNEL_MAX_MISSED_PONGS, 1)
    this.relayAuthToken = options.relayAuthToken?.trim() || null
    this.relayTarget = this.relayAuthToken
      ? {
        daemon_id: options.targetDaemonId?.trim() || null,
        daemon_alias: options.targetDaemonAlias?.trim() || null,
      }
      : null
  }

  supportsKernelEvents() {
    return isWebSocketEndpoint(this.socketPath)
  }

  private isRelayMode() {
    return this.relayAuthToken != null
  }

  send<TResponse>(request: unknown): Promise<TResponse> {
    if (isWebSocketEndpoint(this.socketPath)) {
      return this.sendWebSocket(request)
    }
    return this.sendLocalSocket(request)
  }

  async subscribeToKernelEvents(sessionId: string, attachmentId: string): Promise<void> {
    if (!this.supportsKernelEvents()) {
      return
    }
    const previousSubscription = this.activeKernelSubscription
    const resumeFromEventId =
      previousSubscription?.sessionId === sessionId
        && previousSubscription.attachmentId === attachmentId
        ? this.lastReceivedEventId
        : null
    if (resumeFromEventId == null) {
      this.lastReceivedEventId = null
    }
    this.activeKernelSubscription = {
      sessionId,
      attachmentId,
      scope: "session",
      relaySubscriptionId: this.isRelayMode() ? randomUUID() : null,
      relayPrivateKey: null,
    }
    try {
      if (this.isRelayMode()) {
        await this.sendRelaySubscribe(sessionId, attachmentId, resumeFromEventId)
      } else {
        await this.sendWebSocket<Record<string, unknown>>({
          __kernel_transport: {
            type: "subscribe",
            session_id: sessionId,
            attachment_id: attachmentId,
            resume_from_event_id: resumeFromEventId,
          },
        }, "event")
      }
      this.clearReconnectState()
      this.markKernelEventReceived()
    } catch (error) {
      this.scheduleReconnect()
      throw error
    }
  }

  async subscribeToWaitingRoomInventory(): Promise<void> {
    if (!this.supportsKernelEvents()) {
      return
    }
    const sessionId = "__waiting_room_inventory__"
    const attachmentId = "__waiting_room_inventory__"
    const previousSubscription = this.activeKernelSubscription
    const resumeFromEventId =
      previousSubscription?.scope === "waiting_room_inventory" ? this.lastReceivedEventId : null
    if (resumeFromEventId == null) {
      this.lastReceivedEventId = null
    }
    this.activeKernelSubscription = {
      sessionId,
      attachmentId,
      scope: "waiting_room_inventory",
      relaySubscriptionId: this.isRelayMode() ? randomUUID() : null,
      relayPrivateKey: null,
    }
    try {
      if (this.isRelayMode()) {
        await this.sendRelaySubscribe(sessionId, attachmentId, resumeFromEventId, "waiting_room_inventory")
      } else {
        await this.sendWebSocket<Record<string, unknown>>({
          __kernel_transport: {
            type: "subscribe",
            session_id: sessionId,
            attachment_id: attachmentId,
            subscription_scope: "waiting_room_inventory",
            resume_from_event_id: resumeFromEventId,
          },
        }, "event")
      }
      this.clearReconnectState()
      this.markKernelEventReceived()
    } catch (error) {
      this.scheduleReconnect()
      throw error
    }
  }

  async unsubscribeFromKernelEvents(): Promise<void> {
    if (!this.supportsKernelEvents()) {
      return
    }
    const subscription = this.activeKernelSubscription
    this.activeKernelSubscription = null
    this.clearReconnectState()
    this.clearKernelEventWatchdog()
    const socket = this.getWebSocket("event")
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      return
    }
    if (this.isRelayMode()) {
      if (!subscription?.relaySubscriptionId || !subscription.relayPrivateKey) {
        return
      }
      await this.sendRelayUnsubscribe(subscription.relaySubscriptionId, subscription.relayPrivateKey)
    } else {
      await this.sendWebSocket<Record<string, unknown>>({
        __kernel_transport: {
          type: "unsubscribe",
        },
      }, "event")
    }
  }

  async restartKernelEventStream(): Promise<void> {
    if (!this.supportsKernelEvents() || !this.activeKernelSubscription) {
      return
    }
    this.clearReconnectState()
    this.clearKernelEventWatchdog()
    this.clearKernelHeartbeat("event")
    const socket = this.getWebSocket("event")
    if (socket && socket.readyState !== WebSocket.CLOSED) {
      this.suppressNextEventCloseEvent = true
      socket.terminate()
      this.setWebSocket("event", null)
      this.setWebSocketConnectPromise("event", null)
    }
    this.scheduleReconnect(25)
  }

  onKernelEvent(handler: (event: KernelEvent) => void) {
    this.eventHandlers.add(handler)
    return () => {
      this.eventHandlers.delete(handler)
    }
  }

  async close(): Promise<void> {
    this.activeKernelSubscription = null
    this.clearReconnectState()
    this.clearKernelEventWatchdog()
    this.clearKernelHeartbeat("control")
    this.clearKernelHeartbeat("event")
    this.controlRelayDaemonPublicKey = null
    this.eventRelayDaemonPublicKey = null
    await Promise.all([
      this.closeWebSocket("control"),
      this.closeWebSocket("event"),
    ])
  }

  private sendLocalSocket<TResponse>(request: unknown): Promise<TResponse> {
    return new Promise<TResponse>((resolve, reject) => {
      const socket = net.createConnection(this.socketPath)
      const chunks: Buffer[] = []
      let settled = false

      const fail = (operation: string, error: unknown) => {
        if (settled) {
          return
        }
        settled = true
        socket.destroy()
        reject(new LocalIpcError(operation, error instanceof Error ? error.message : String(error)))
      }

      const succeed = (value: TResponse) => {
        if (settled) {
          return
        }
        settled = true
        socket.destroy()
        resolve(value)
      }

      socket.setTimeout(IPC_TIMEOUT_MS)
      socket.once("timeout", () => fail("handle local response", "timed out"))
      socket.once("error", (error) => fail("connect local socket", error))

      socket.once("connect", () => {
        let payload: Buffer
        try {
          payload = Buffer.from(JSON.stringify(request), "utf8")
        } catch (error) {
          fail("serialize local request", error)
          return
        }

        const frame = Buffer.allocUnsafe(4 + payload.length)
        frame.writeUInt32BE(payload.length, 0)
        payload.copy(frame, 4)

        socket.write(frame, (error) => {
          if (error) {
            fail("write local request", error)
          }
        })
      })

      socket.on("data", (chunk) => {
        chunks.push(chunk)
      })

      socket.once("end", () => {
        const buffer = Buffer.concat(chunks)
        if (buffer.length < 4) {
          fail("read local response header", "response header was truncated")
          return
        }

        const payloadLength = buffer.readUInt32BE(0)
        const payload = buffer.subarray(4)
        if (payload.length < payloadLength) {
          fail("read local response body", "response body was truncated")
          return
        }

        let envelope: IpcEnvelope<TResponse>
        try {
          envelope = JSON.parse(payload.subarray(0, payloadLength).toString("utf8")) as IpcEnvelope<TResponse>
        } catch (error) {
          fail("decode local response", error)
          return
        }

        if (envelope.error) {
          fail("handle local response", envelope.error)
          return
        }
        if (envelope.response == null) {
          fail("handle local response", "response envelope was empty")
          return
        }

        succeed(envelope.response)
      })
    })
  }

  private async sendWebSocket<TResponse>(request: unknown, lane: KernelSocketLane = "control"): Promise<TResponse> {
    const socket = await this.ensureWebSocket(lane)
    const requestId = randomUUID()

    return new Promise<TResponse>((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(requestId)
        reject(new LocalIpcError("handle kernel response", "timed out", "request_timeout", true))
      }, IPC_TIMEOUT_MS)

      this.pending.set(requestId, {
        resolve: resolve as (value: unknown) => void,
        reject,
        timeout,
        relayPrivateKey: null,
        lane,
      })

      try {
        const relayRequest = this.isRelayMode()
          ? normalizeRelayRequest(requestId, request, this.relayTarget, this.getRelayDaemonPublicKey(lane))
          : null
        if (relayRequest) {
          const pending = this.pending.get(requestId)
          if (pending) {
            pending.relayPrivateKey = relayRequest.privateKey
          }
        }
        const payload = relayRequest
          ? relayRequest.frame
          : normalizeWebSocketRequest(requestId, request)
        socket.send(JSON.stringify(payload))
      } catch (error) {
        clearTimeout(timeout)
        this.pending.delete(requestId)
        reject(new LocalIpcError("write kernel request", error instanceof Error ? error.message : String(error), "write_failed", true))
      }
    })
  }

  private async sendRelaySubscribe(
    sessionId: string,
    attachmentId: string,
    resumeFromEventId: number | null,
    subscriptionScope?: string,
  ): Promise<void> {
    const lane: KernelSocketLane = "event"
    const socket = await this.ensureWebSocket(lane)
    const requestId = randomUUID()
    const subscription = this.activeKernelSubscription
    if (!subscription?.relaySubscriptionId) {
      throw new LocalIpcError("write relay subscribe", "relay subscription state is missing")
    }
    const subscriptionId = subscription.relaySubscriptionId
    const keypair = createRelayKeypair()
    subscription.relayPrivateKey = keypair.privateKey

    await new Promise<void>((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(requestId)
        reject(new LocalIpcError("handle kernel response", "timed out", "request_timeout", true))
      }, IPC_TIMEOUT_MS)

      this.pending.set(requestId, {
        resolve: () => resolve(),
        reject,
        timeout,
        relayPrivateKey: keypair.privateKey,
        lane,
      })

      try {
        const frame: RelaySubscribeFrame = {
          kind: "client_subscribe",
          request_id: requestId,
          subscription_id: subscriptionId,
          target: requireRelayTarget(this.relayTarget),
          session_id: sessionId,
          attachment_id: attachmentId,
          client_public_key: keypair.publicKeyBase64,
          resume_from_event_id: resumeFromEventId,
        }
        if (subscriptionScope) {
          frame.subscription_scope = subscriptionScope
        }
        socket.send(JSON.stringify(frame))
      } catch (error) {
        clearTimeout(timeout)
        this.pending.delete(requestId)
        reject(new LocalIpcError("write relay subscribe", error instanceof Error ? error.message : String(error), "write_failed", true))
      }
    })
  }

  private async sendRelayUnsubscribe(subscriptionId: string, privateKey: Buffer): Promise<void> {
    const lane: KernelSocketLane = "event"
    const socket = await this.ensureWebSocket(lane)
    const requestId = randomUUID()
    const publicKeyBase64 = relayPublicKeyFromPrivateKey(privateKey)

    await new Promise<void>((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(requestId)
        reject(new LocalIpcError("handle kernel response", "timed out", "request_timeout", true))
      }, IPC_TIMEOUT_MS)

      this.pending.set(requestId, {
        resolve: () => resolve(),
        reject,
        timeout,
        relayPrivateKey: privateKey,
        lane,
      })

      try {
        const frame: RelayUnsubscribeFrame = {
          kind: "client_unsubscribe",
          request_id: requestId,
          subscription_id: subscriptionId,
          client_public_key: publicKeyBase64,
        }
        socket.send(JSON.stringify(frame))
      } catch (error) {
        clearTimeout(timeout)
        this.pending.delete(requestId)
        reject(new LocalIpcError("write relay unsubscribe", error instanceof Error ? error.message : String(error), "write_failed", true))
      }
    })
  }

  private async ensureWebSocket(lane: KernelSocketLane = "control"): Promise<WebSocket> {
    const existing = this.getWebSocket(lane)
    if (existing?.readyState === WebSocket.OPEN) {
      return existing
    }
    const connectPromise = this.getWebSocketConnectPromise(lane)
    if (connectPromise) {
      return connectPromise
    }

    const nextConnectPromise = new Promise<WebSocket>((resolve, reject) => {
      const socket = new WebSocket(this.socketPath)
      let settled = false

      const fail = (operation: string, error: unknown) => {
        if (settled) {
          return
        }
        settled = true
        this.setWebSocketConnectPromise(lane, null)
        reject(new LocalIpcError(operation, formatTransportError(error, this.socketPath)))
      }

      socket.once("open", () => {
        const finalizeOpen = () => {
          settled = true
          this.setWebSocket(lane, socket)
          this.setWebSocketConnectPromise(lane, null)
          this.setSuppressNextCloseEvent(lane, false)
          this.startKernelHeartbeat(socket, lane)
          socket.on("message", (data: WebSocket.RawData) => {
            this.handleWebSocketMessage(data, lane)
          })
          socket.on("pong", () => {
            this.setMissedKernelPongs(lane, 0)
          })
          socket.once("close", (code: number, reason: Buffer) => {
            const suppressed = this.getSuppressNextCloseEvent(lane)
            this.setSuppressNextCloseEvent(lane, false)
            this.rejectPending("kernel websocket closed", lane)
            this.setWebSocket(lane, null)
            this.setRelayDaemonPublicKey(lane, null)
            this.clearKernelHeartbeat(lane)
            const closeMessage = reason.length > 0
              ? reason.toString("utf8")
              : `kernel websocket closed${code ? ` (${code})` : ""}`
            if (!suppressed) {
              this.emitSyntheticEvent({
                event: "transport_closed",
                message: closeMessage,
              })
              if (lane === "event") {
                this.scheduleReconnect()
              }
            }
          })
          socket.on("error", (error: unknown) => {
            const message = formatTransportError(error, this.socketPath)
            const suppressed = this.getSuppressNextCloseEvent(lane)
            this.setSuppressNextCloseEvent(lane, false)
            this.rejectPending(message, lane)
            this.setWebSocket(lane, null)
            this.setRelayDaemonPublicKey(lane, null)
            this.clearKernelHeartbeat(lane)
            if (!suppressed) {
              this.emitSyntheticEvent({
                event: "transport_closed",
                message,
              })
              if (lane === "event") {
                this.scheduleReconnect()
              }
            }
          })
          resolve(socket)
        }

        if (!this.isRelayMode()) {
          socket.off("error", handleConnectError)
          finalizeOpen()
          return
        }

        const handleRelayHandshakeMessage = (data: WebSocket.RawData) => {
          let frame: RelayConnectedFrame | RelayCloseFrame
          try {
            frame = JSON.parse(String(data)) as RelayConnectedFrame | RelayCloseFrame
          } catch (error) {
            fail("connect relay transport", error)
            return
          }
          if (frame.kind === "client_connected") {
            if (!frame.daemon_public_key) {
              fail("connect relay transport", "relay did not provide daemon public key")
              return
            }
            this.setRelayDaemonPublicKey(lane, frame.daemon_public_key)
            socket.off("message", handleRelayHandshakeMessage)
            socket.off("error", handleConnectError)
            finalizeOpen()
            return
          }
          if (frame.kind === "close") {
            fail("connect relay transport", frame.reason)
            return
          }
          fail("connect relay transport", "unexpected relay handshake frame")
        }

        socket.on("message", handleRelayHandshakeMessage)
        try {
          socket.send(JSON.stringify(buildRelayConnectFrame(this.relayAuthToken, this.relayTarget)))
        } catch (error) {
          socket.off("message", handleRelayHandshakeMessage)
          fail("write relay connect frame", error)
        }
      })

      const handleConnectError = (error: unknown) => fail("connect kernel websocket", error)
      socket.on("error", handleConnectError)
    })

    this.setWebSocketConnectPromise(lane, nextConnectPromise)
    return nextConnectPromise
  }

  private handleWebSocketMessage(data: WebSocket.RawData, lane: KernelSocketLane) {
    let frame:
      | KernelTransportResponseFrame<unknown>
      | KernelTransportEventFrame<KernelEvent>
      | RelayResponseFrame<unknown>
      | RelayEventFrame
      | RelayCloseFrame
    try {
      frame = JSON.parse(String(data)) as
        | KernelTransportResponseFrame<unknown>
        | KernelTransportEventFrame<KernelEvent>
        | RelayResponseFrame<unknown>
        | RelayEventFrame
        | RelayCloseFrame
    } catch (error) {
      this.rejectPending(error instanceof Error ? error.message : String(error), lane)
      return
    }

    if ("type" in frame && frame.type === "event") {
      this.lastReceivedEventId = frame.event_id
      this.markKernelEventReceived()
      for (const handler of this.eventHandlers) {
        handler(frame.event)
      }
      return
    }

    if ("kind" in frame && frame.kind === "close") {
      this.rejectPending(frame.reason, lane)
      return
    }

    if ("kind" in frame && frame.kind === "client_event") {
      const subscription = this.activeKernelSubscription
      if (!subscription?.relayPrivateKey || subscription.relaySubscriptionId !== frame.subscription_id) {
        return
      }
      try {
        const decrypted = decryptRelayPayload(subscription.relayPrivateKey, frame.encrypted_event)
        const event = JSON.parse(decrypted) as KernelEvent
        this.lastReceivedEventId = frame.event_id
        this.markKernelEventReceived()
        this.emitSyntheticEvent(event)
      } catch (error) {
        this.rejectPending(error instanceof Error ? error.message : String(error), lane)
      }
      return
    }

    const requestId = "type" in frame ? frame.request_id : frame.request_id
    const pending = this.pending.get(requestId)
    if (!pending) {
      return
    }
    clearTimeout(pending.timeout)
    this.pending.delete(requestId)

    if (frame.error) {
      pending.reject(new LocalIpcError("handle kernel response", frame.error.message, frame.error.code, frame.error.retryable))
      return
    }
    if ("kind" in frame) {
      if (!pending.relayPrivateKey) {
        pending.reject(new LocalIpcError("handle kernel response", "missing relay request key"))
        return
      }
      if (frame.encrypted_response == null) {
        pending.reject(new LocalIpcError("handle kernel response", "response envelope was empty"))
        return
      }
      try {
        const decrypted = decryptRelayPayload(pending.relayPrivateKey, frame.encrypted_response)
        pending.resolve(JSON.parse(decrypted) as unknown)
      } catch (error) {
        pending.reject(new LocalIpcError("handle kernel response", error instanceof Error ? error.message : String(error)))
      }
      return
    }
    if (frame.response == null) {
      pending.reject(new LocalIpcError("handle kernel response", "response envelope was empty"))
      return
    }

    pending.resolve(frame.response)
  }

  private rejectPending(message: string, lane?: KernelSocketLane) {
    const pendingEntries = Array.from(this.pending.entries())
      .filter(([, pending]) => !lane || pending.lane === lane)
    for (const [requestId] of pendingEntries) {
      this.pending.delete(requestId)
    }
    for (const [, pending] of pendingEntries) {
      clearTimeout(pending.timeout)
      pending.reject(new LocalIpcError("kernel websocket", message, "connection_closed", true))
    }
  }

  private emitSyntheticEvent(event: KernelEvent) {
    for (const handler of this.eventHandlers) {
      handler(event)
    }
  }

  private clearReconnectState() {
    if (this.reconnectTimeout) {
      clearTimeout(this.reconnectTimeout)
      this.reconnectTimeout = null
    }
    this.reconnectDelayMs = 250
  }

  private markKernelEventReceived() {
    this.lastKernelEventAtMs = Date.now()
    this.armKernelEventWatchdog()
  }

  private armKernelEventWatchdog() {
    this.clearKernelEventWatchdog()
    if (!this.kernelEventStaleMs || !this.activeKernelSubscription || this.eventHandlers.size === 0) {
      return
    }
    this.kernelEventWatchdog = setTimeout(() => {
      const elapsedMs = Date.now() - this.lastKernelEventAtMs
      if (!this.activeKernelSubscription || this.eventHandlers.size === 0) {
        return
      }
      if (elapsedMs < this.kernelEventStaleMs) {
        this.armKernelEventWatchdog()
        return
      }
      this.emitSyntheticEvent({
        event: "transport_closed",
        message: `kernel event stream stalled for ${elapsedMs}ms; reconnecting`,
      })
      void this.restartKernelEventStream()
    }, this.kernelEventStaleMs)
  }

  private clearKernelEventWatchdog() {
    if (this.kernelEventWatchdog) {
      clearTimeout(this.kernelEventWatchdog)
      this.kernelEventWatchdog = null
    }
  }

  private startKernelHeartbeat(socket: WebSocket, lane: KernelSocketLane) {
    this.clearKernelHeartbeat(lane)
    this.setMissedKernelPongs(lane, 0)
    const heartbeat = setInterval(() => {
      if (socket !== this.getWebSocket(lane) || socket.readyState !== WebSocket.OPEN) {
        this.clearKernelHeartbeat(lane)
        return
      }
      if (this.getMissedKernelPongs(lane) >= this.kernelMaxMissedPongs) {
        if (lane === "event") {
          this.emitSyntheticEvent({
            event: "transport_closed",
            message: "kernel websocket heartbeat missed; reconnecting",
          })
        }
        this.setSuppressNextCloseEvent(lane, true)
        socket.terminate()
        this.setWebSocket(lane, null)
        this.setRelayDaemonPublicKey(lane, null)
        if (lane === "event") {
          this.scheduleReconnect()
        }
        return
      }
      this.setMissedKernelPongs(lane, this.getMissedKernelPongs(lane) + 1)
      try {
        socket.ping()
      } catch {
        if (lane === "event") {
          this.emitSyntheticEvent({
            event: "transport_closed",
            message: "kernel websocket heartbeat failed; reconnecting",
          })
        }
        this.setSuppressNextCloseEvent(lane, true)
        socket.terminate()
        this.setWebSocket(lane, null)
        this.setRelayDaemonPublicKey(lane, null)
        if (lane === "event") {
          this.scheduleReconnect()
        }
      }
    }, this.kernelPingIntervalMs)
    if (lane === "control") {
      this.controlHeartbeat = heartbeat
    } else {
      this.eventHeartbeat = heartbeat
    }
  }

  private clearKernelHeartbeat(lane: KernelSocketLane) {
    const heartbeat = lane === "control" ? this.controlHeartbeat : this.eventHeartbeat
    if (heartbeat) {
      clearInterval(heartbeat)
      if (lane === "control") {
        this.controlHeartbeat = null
      } else {
        this.eventHeartbeat = null
      }
    }
    this.setMissedKernelPongs(lane, 0)
  }

  private scheduleReconnect(delayMs = this.reconnectDelayMs) {
    if (!this.activeKernelSubscription || this.eventHandlers.size === 0 || this.reconnectTimeout) {
      return
    }

    this.reconnectTimeout = setTimeout(() => {
      this.reconnectTimeout = null
      void this.resumeKernelSubscription()
    }, delayMs)
    this.reconnectDelayMs = Math.min(Math.max(delayMs * 2, 250), 5_000)
  }

  private async resumeKernelSubscription() {
    const subscription = this.activeKernelSubscription
    if (!subscription || this.eventHandlers.size === 0) {
      return
    }

    try {
      if (this.isRelayMode()) {
        await this.sendRelaySubscribe(
          subscription.sessionId,
          subscription.attachmentId,
          this.lastReceivedEventId,
          subscription.scope === "waiting_room_inventory" ? "waiting_room_inventory" : undefined,
        )
      } else {
        await this.sendWebSocket<Record<string, unknown>>({
          __kernel_transport: {
            type: "subscribe",
            session_id: subscription.sessionId,
            attachment_id: subscription.attachmentId,
            subscription_scope: subscription.scope === "waiting_room_inventory" ? "waiting_room_inventory" : undefined,
            resume_from_event_id: this.lastReceivedEventId,
          },
        }, "event")
      }
      this.clearReconnectState()
      this.markKernelEventReceived()
      this.emitSyntheticEvent({
        event: "transport_resumed",
        session_id: subscription.sessionId,
        resumed_from_event_id: this.lastReceivedEventId,
      })
    } catch {
      this.scheduleReconnect()
    }
  }

  private getWebSocket(lane: KernelSocketLane) {
    return lane === "control" ? this.controlWebsocket : this.eventWebsocket
  }

  private setWebSocket(lane: KernelSocketLane, socket: WebSocket | null) {
    if (lane === "control") {
      this.controlWebsocket = socket
    } else {
      this.eventWebsocket = socket
    }
  }

  private getWebSocketConnectPromise(lane: KernelSocketLane) {
    return lane === "control" ? this.controlWebsocketConnectPromise : this.eventWebsocketConnectPromise
  }

  private setWebSocketConnectPromise(lane: KernelSocketLane, promise: Promise<WebSocket> | null) {
    if (lane === "control") {
      this.controlWebsocketConnectPromise = promise
    } else {
      this.eventWebsocketConnectPromise = promise
    }
  }

  private getRelayDaemonPublicKey(lane: KernelSocketLane) {
    return lane === "control" ? this.controlRelayDaemonPublicKey : this.eventRelayDaemonPublicKey
  }

  private setRelayDaemonPublicKey(lane: KernelSocketLane, publicKey: string | null) {
    if (lane === "control") {
      this.controlRelayDaemonPublicKey = publicKey
    } else {
      this.eventRelayDaemonPublicKey = publicKey
    }
  }

  private getSuppressNextCloseEvent(lane: KernelSocketLane) {
    return lane === "control" ? this.suppressNextControlCloseEvent : this.suppressNextEventCloseEvent
  }

  private setSuppressNextCloseEvent(lane: KernelSocketLane, value: boolean) {
    if (lane === "control") {
      this.suppressNextControlCloseEvent = value
    } else {
      this.suppressNextEventCloseEvent = value
    }
  }

  private getMissedKernelPongs(lane: KernelSocketLane) {
    return lane === "control" ? this.missedControlPongs : this.missedEventPongs
  }

  private setMissedKernelPongs(lane: KernelSocketLane, value: number) {
    if (lane === "control") {
      this.missedControlPongs = value
    } else {
      this.missedEventPongs = value
    }
  }

  private async closeWebSocket(lane: KernelSocketLane): Promise<void> {
    const socket = this.getWebSocket(lane)
    this.setWebSocket(lane, null)
    this.setWebSocketConnectPromise(lane, null)
    this.setRelayDaemonPublicKey(lane, null)
    if (!socket || socket.readyState === WebSocket.CLOSED) {
      return
    }

    await new Promise<void>((resolve) => {
      let settled = false
      let timeout: ReturnType<typeof setTimeout> | undefined
      const finish = () => {
        if (settled) {
          return
        }
        settled = true
        if (timeout) {
          clearTimeout(timeout)
        }
        resolve()
      }
      timeout = setTimeout(() => {
        socket.terminate()
        finish()
      }, IPC_WEBSOCKET_CLOSE_TIMEOUT_MS)
      this.setSuppressNextCloseEvent(lane, true)
      socket.once("close", finish)
      socket.close()
    })
  }
}

function buildRelayConnectFrame(authToken: string | null, target: RelayTarget | null): RelayConnectFrame {
  if (!authToken) {
    throw new Error("relay auth token is required")
  }
  if (!target?.daemon_id && !target?.daemon_alias) {
    throw new Error("relay target daemon id or alias is required")
  }
  return {
    kind: "client_connect",
    auth_token: authToken,
    target: target ?? {},
  }
}

function normalizeRelayRequest(
  requestId: string,
  request: unknown,
  target: RelayTarget | null,
  daemonPublicKey: string | null,
): { frame: RelayRequestFrame; privateKey: Buffer } {
  const resolvedTarget = requireRelayTarget(target)
  if (!daemonPublicKey) {
    throw new Error("relay daemon public key is required")
  }
  const plaintext = Buffer.from(JSON.stringify(request), "utf8")
  const { privateKey, payload } = encryptRelayPayload(daemonPublicKey, plaintext)
  return {
    frame: {
      kind: "client_request",
      request_id: requestId,
      target: resolvedTarget,
      encrypted_request: payload,
    },
    privateKey,
  }
}

function isWebSocketEndpoint(value: string) {
  return value.startsWith("ws://") || value.startsWith("wss://")
}

function formatTransportError(error: unknown, endpoint: string): string {
  const message = extractTransportErrorMessage(error)
  if (message) {
    return message
  }
  return `websocket error at ${endpoint}`
}

function extractTransportErrorMessage(error: unknown): string | null {
  if (error instanceof Error && error.message.trim()) {
    return error.message
  }
  if (typeof error === "string" && error.trim()) {
    return error
  }
  if (!error || typeof error !== "object") {
    return null
  }

  const fields = error as Record<string, unknown>
  const nested = extractTransportErrorMessage(fields.error)
  if (nested) {
    return nested
  }
  if (typeof fields.message === "string" && fields.message.trim()) {
    return fields.message
  }
  if (typeof fields.reason === "string" && fields.reason.trim()) {
    return fields.reason
  }
  if (typeof fields.code === "string" && fields.code.trim()) {
    return fields.code
  }
  if (typeof fields.type === "string" && fields.type.trim() && fields.type !== "error") {
    return `websocket ${fields.type}`
  }
  return null
}

function normalizeWebSocketRequest(requestId: string, request: unknown) {
  const transportRequest = extractTransportRequest(request)
  if (transportRequest?.type === "subscribe") {
    return {
      type: "subscribe",
      request_id: requestId,
      session_id: transportRequest.session_id,
      attachment_id: transportRequest.attachment_id,
      subscription_scope: transportRequest.subscription_scope ?? null,
      resume_from_event_id: transportRequest.resume_from_event_id ?? null,
    }
  }
  if (transportRequest?.type === "unsubscribe") {
    return {
      type: "unsubscribe",
      request_id: requestId,
    }
  }
  return {
    type: "request",
    request_id: requestId,
    request,
  }
}

function extractTransportRequest(request: unknown):
  | { type: "subscribe"; session_id: string; attachment_id: string; subscription_scope?: string | null; resume_from_event_id?: number | null }
  | { type: "unsubscribe" }
  | null {
  if (!request || typeof request !== "object") {
    return null
  }
  const value = (request as { __kernel_transport?: unknown }).__kernel_transport
  if (!value || typeof value !== "object") {
    return null
  }
  const transport = value as Record<string, unknown>
  if (
    transport.type === "subscribe"
    && typeof transport.session_id === "string"
    && typeof transport.attachment_id === "string"
  ) {
    return {
      type: "subscribe",
      session_id: transport.session_id,
      attachment_id: transport.attachment_id,
      subscription_scope: typeof transport.subscription_scope === "string" ? transport.subscription_scope : null,
      resume_from_event_id:
        typeof transport.resume_from_event_id === "number" ? transport.resume_from_event_id : null,
    }
  }
  if (transport.type === "unsubscribe") {
    return { type: "unsubscribe" }
  }
  return null
}

const RELAY_NONCE_LEN = 12
const RELAY_TAG_LEN = 16
const RELAY_INFO = Buffer.from("arroba-relay-v1", "utf8")

function encryptRelayPayload(
  peerPublicKeyBase64: string,
  plaintext: Buffer,
): { privateKey: Buffer; payload: EncryptedRelayPayload } {
  const ecdh = createECDH("prime256v1")
  const publicKey = ecdh.generateKeys()
  const privateKey = ecdh.getPrivateKey()
  const sharedSecret = ecdh.computeSecret(Buffer.from(peerPublicKeyBase64, "base64"))
  const key = deriveRelayKey(sharedSecret)
  const nonce = randomBytes(RELAY_NONCE_LEN)
  const cipher = createCipheriv("aes-256-gcm", key, nonce)
  const ciphertext = Buffer.concat([cipher.update(plaintext), cipher.final(), cipher.getAuthTag()])
  return {
    privateKey,
    payload: {
      sender_public_key: publicKey.toString("base64"),
      nonce: nonce.toString("base64"),
      ciphertext: ciphertext.toString("base64"),
    },
  }
}

function decryptRelayPayload(privateKey: Buffer, payload: EncryptedRelayPayload): string {
  const ecdh = createECDH("prime256v1")
  ecdh.setPrivateKey(privateKey)
  const sharedSecret = ecdh.computeSecret(Buffer.from(payload.sender_public_key, "base64"))
  const key = deriveRelayKey(sharedSecret)
  const nonce = Buffer.from(payload.nonce, "base64")
  if (nonce.length !== RELAY_NONCE_LEN) {
    throw new Error("invalid relay nonce")
  }
  const ciphertext = Buffer.from(payload.ciphertext, "base64")
  if (ciphertext.length < RELAY_TAG_LEN) {
    throw new Error("invalid relay ciphertext")
  }
  const body = ciphertext.subarray(0, ciphertext.length - RELAY_TAG_LEN)
  const tag = ciphertext.subarray(ciphertext.length - RELAY_TAG_LEN)
  const decipher = createDecipheriv("aes-256-gcm", key, nonce)
  decipher.setAuthTag(tag)
  const plaintext = Buffer.concat([decipher.update(body), decipher.final()])
  return plaintext.toString("utf8")
}

function deriveRelayKey(sharedSecret: Buffer): Buffer {
  return Buffer.from(hkdfSync("sha256", sharedSecret, Buffer.alloc(0), RELAY_INFO, 32))
}

function createRelayKeypair(): { privateKey: Buffer; publicKeyBase64: string } {
  const ecdh = createECDH("prime256v1")
  const publicKey = ecdh.generateKeys()
  return {
    privateKey: ecdh.getPrivateKey(),
    publicKeyBase64: publicKey.toString("base64"),
  }
}

function relayPublicKeyFromPrivateKey(privateKey: Buffer): string {
  const ecdh = createECDH("prime256v1")
  ecdh.setPrivateKey(privateKey)
  return ecdh.getPublicKey().toString("base64")
}

function requireRelayTarget(target: RelayTarget | null): RelayTarget {
  if (!target?.daemon_id && !target?.daemon_alias) {
    throw new Error("relay target daemon id or alias is required")
  }
  return target
}
