import { randomUUID } from "node:crypto"

import WebSocket from "ws"

import type { KernelEvent } from "./kernel-events.js"
import type {
  IpcEnvelope,
  KernelSocketLane,
  KernelTransportEventFrame,
  KernelTransportResponseFrame,
  RelayCloseFrame,
  RelayConnectedFrame,
  RelayEventFrame,
  RelayResponseFrame,
  RelayTarget,
} from "./kernel-transport-frames.js"
import { normalizeWebSocketRequest } from "./kernel-transport-requests.js"
import {
  buildKernelSubscriptionTransportRequest,
  createKernelSessionSubscriptionStart,
  createWaitingRoomInventorySubscriptionStart,
  kernelSubscriptionScopeValue,
  type KernelSubscriptionState,
} from "./kernel-subscriptions.js"
import { LocalIpcError } from "./local-ipc-error.js"
import { sendLocalSocketRequest } from "./local-socket-transport.js"
import { createRelayKeypair, decryptRelayPayload } from "./relay-crypto.js"
import {
  buildRelayConnectFrame,
  buildRelaySubscribeFrame,
  buildRelayUnsubscribeFrame,
  normalizeRelayRequest,
} from "./relay-transport.js"
import { KernelPendingRequestRegistry } from "./websocket-pending-requests.js"
import { formatTransportError, isWebSocketEndpoint } from "./websocket-transport-diagnostics.js"

// Slice start can cold-build the managed Linux image before returning the
// worker kernel endpoint. Keep the control request open long enough for first
// run provisioning while lifecycle progress remains request/response based.
const IPC_TIMEOUT_MS = 600_000
const DEFAULT_KERNEL_EVENT_STALE_MS = 0
const DEFAULT_KERNEL_PING_INTERVAL_MS = 5_000
const DEFAULT_KERNEL_MAX_MISSED_PONGS = 2
const IPC_WEBSOCKET_CLOSE_TIMEOUT_MS = 1_000
const IPC_CLIENT_CLOSE_TIMEOUT_MS = 1_500
const KERNEL_RECONNECT_BASE_DELAY_MS = 250
const KERNEL_RECONNECT_MAX_DELAY_MS = 5_000
const KERNEL_RECONNECT_JITTER_MS = 250
const KERNEL_CONTROL_REQUEST_RETRY_DEADLINE_MS = 60_000

export type { KernelEvent } from "./kernel-events.js"
export { LocalIpcError } from "./local-ipc-error.js"

type LocalIpcClientOptions = {
  relayAuthToken?: string | undefined
  targetDaemonId?: string | undefined
  targetDaemonAlias?: string | undefined
  kernelEventStaleMs?: number | undefined
  kernelPingIntervalMs?: number | undefined
  kernelMaxMissedPongs?: number | undefined
  reconnectJitterMs?: number | undefined
  reconnectRandom?: (() => number) | undefined
  controlRequestRetryDeadlineMs?: number | undefined
}

export class LocalIpcClient {
  readonly socketPath: string
  private readonly relayAuthToken: string | null
  private readonly relayTarget: RelayTarget | null
  private controlWebsocket: WebSocket | null = null
  private eventWebsocket: WebSocket | null = null
  private controlWebsocketConnectPromise: Promise<WebSocket> | null = null
  private eventWebsocketConnectPromise: Promise<WebSocket> | null = null
  private readonly pendingRequests = new KernelPendingRequestRegistry(IPC_TIMEOUT_MS)
  private eventHandlers = new Set<(event: KernelEvent) => void>()
  private activeKernelSubscription: KernelSubscriptionState | null = null
  private reconnectTimeout: NodeJS.Timeout | null = null
  private reconnectDelayMs = 250
  private lastReceivedEventId: number | null = null
  private lastKernelEventAtMs = 0
  private kernelEventWatchdog: NodeJS.Timeout | null = null
  private controlHeartbeat: NodeJS.Timeout | null = null
  private eventHeartbeat: NodeJS.Timeout | null = null
  private readonly reconnectJitterMs: number
  private readonly reconnectRandom: () => number
  private readonly controlRequestRetryDeadlineMs: number
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
    this.reconnectJitterMs = Math.max(options.reconnectJitterMs ?? KERNEL_RECONNECT_JITTER_MS, 0)
    this.reconnectRandom = options.reconnectRandom ?? Math.random
    this.controlRequestRetryDeadlineMs = Math.max(
      options.controlRequestRetryDeadlineMs ?? KERNEL_CONTROL_REQUEST_RETRY_DEADLINE_MS,
      0,
    )
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
    const start = createKernelSessionSubscriptionStart({
      previous: this.activeKernelSubscription,
      lastReceivedEventId: this.lastReceivedEventId,
      sessionId,
      attachmentId,
      relaySubscriptionId: this.isRelayMode() ? randomUUID() : null,
    })
    if (start.resetLastReceivedEventId) {
      this.lastReceivedEventId = null
    }
    this.activeKernelSubscription = start.subscription
    try {
      if (this.isRelayMode()) {
        await this.sendRelaySubscribe(sessionId, attachmentId, start.resumeFromEventId)
      } else {
        await this.sendWebSocket<Record<string, unknown>>(
          buildKernelSubscriptionTransportRequest(start.subscription, start.resumeFromEventId),
          "event",
        )
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
    const start = createWaitingRoomInventorySubscriptionStart({
      previous: this.activeKernelSubscription,
      lastReceivedEventId: this.lastReceivedEventId,
      relaySubscriptionId: this.isRelayMode() ? randomUUID() : null,
    })
    if (start.resetLastReceivedEventId) {
      this.lastReceivedEventId = null
    }
    this.activeKernelSubscription = start.subscription
    try {
      if (this.isRelayMode()) {
        await this.sendRelaySubscribe(
          start.subscription.sessionId,
          start.subscription.attachmentId,
          start.resumeFromEventId,
          kernelSubscriptionScopeValue(start.subscription),
        )
      } else {
        await this.sendWebSocket<Record<string, unknown>>(
          buildKernelSubscriptionTransportRequest(start.subscription, start.resumeFromEventId),
          "event",
        )
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
    this.clearRuntimeTransportState("kernel client closed")
    let timedOut = false
    let timeout: ReturnType<typeof setTimeout> | undefined
    await Promise.race([
      Promise.all([
        this.closeWebSocket("control"),
        this.closeWebSocket("event"),
      ]).then(() => undefined),
      new Promise<void>((resolve) => {
        timeout = setTimeout(() => {
          timedOut = true
          resolve()
        }, IPC_CLIENT_CLOSE_TIMEOUT_MS)
      }),
    ])
    if (timeout) {
      clearTimeout(timeout)
    }
    if (timedOut) {
      this.destroy()
    }
  }

  destroy(): void {
    this.clearRuntimeTransportState("kernel client destroyed")
    this.destroyWebSocket("control")
    this.destroyWebSocket("event")
  }

  private clearRuntimeTransportState(pendingMessage: string): void {
    this.activeKernelSubscription = null
    this.clearReconnectState()
    this.clearKernelEventWatchdog()
    this.clearKernelHeartbeat("control")
    this.clearKernelHeartbeat("event")
    this.controlRelayDaemonPublicKey = null
    this.eventRelayDaemonPublicKey = null
    this.rejectPending(pendingMessage)
  }

  private sendLocalSocket<TResponse>(request: unknown): Promise<TResponse> {
    return sendLocalSocketRequest(this.socketPath, request, IPC_TIMEOUT_MS)
  }

  private async sendWebSocket<TResponse>(request: unknown, lane: KernelSocketLane = "control"): Promise<TResponse> {
    const requestId = randomUUID()
    const retryUntilMs = lane === "control"
      ? Date.now() + this.controlRequestRetryDeadlineMs
      : Date.now()
    let retryDelayMs = KERNEL_RECONNECT_BASE_DELAY_MS

    for (;;) {
      let socket: WebSocket
      try {
        socket = await this.ensureWebSocket(lane)
      } catch (error) {
        if (!this.shouldReplayWebSocketRequest(error, lane, retryUntilMs)) {
          throw error
        }
        this.destroyWebSocket(lane)
        retryDelayMs = await this.waitBeforeWebSocketRequestReplay(retryDelayMs, retryUntilMs)
        continue
      }

      const pending = this.pendingRequests.register<TResponse>(requestId, lane)

      try {
        const relayRequest = this.isRelayMode()
          ? normalizeRelayRequest(requestId, request, this.relayTarget, this.getRelayDaemonPublicKey(lane))
          : null
        if (relayRequest) {
          pending.setRelayPrivateKey(relayRequest.privateKey)
        }
        const payload = relayRequest
          ? relayRequest.frame
          : normalizeWebSocketRequest(requestId, request)
        socket.send(JSON.stringify(payload))
      } catch (error) {
        pending.reject(new LocalIpcError("write kernel request", error instanceof Error ? error.message : String(error), "write_failed", true))
      }

      try {
        return await pending.promise
      } catch (error) {
        if (!this.shouldReplayWebSocketRequest(error, lane, retryUntilMs)) {
          throw error
        }
        this.destroyWebSocket(lane)
        retryDelayMs = await this.waitBeforeWebSocketRequestReplay(retryDelayMs, retryUntilMs)
      }
    }
  }

  private shouldReplayWebSocketRequest(error: unknown, lane: KernelSocketLane, retryUntilMs: number): boolean {
    return lane === "control"
      && Date.now() < retryUntilMs
      && error instanceof LocalIpcError
      && error.retryable
      && (error.code === "connection_closed" || error.code === "write_failed")
  }

  private async waitBeforeWebSocketRequestReplay(delayMs: number, retryUntilMs: number): Promise<number> {
    const remainingMs = retryUntilMs - Date.now()
    if (remainingMs <= 0) {
      return delayMs
    }
    const waitMs = Math.min(this.reconnectDelayWithJitter(delayMs), remainingMs)
    if (waitMs > 0) {
      await new Promise((resolve) => setTimeout(resolve, waitMs))
    }
    return this.nextReconnectDelayMs(delayMs)
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

    const pending = this.pendingRequests.register<void>(requestId, lane)
    pending.setRelayPrivateKey(keypair.privateKey)

    try {
      const frame = buildRelaySubscribeFrame({
        requestId,
        subscriptionId,
        target: this.relayTarget,
        sessionId,
        attachmentId,
        clientPublicKey: keypair.publicKeyBase64,
        resumeFromEventId,
        subscriptionScope,
      })
      socket.send(JSON.stringify(frame))
    } catch (error) {
      pending.reject(new LocalIpcError("write relay subscribe", error instanceof Error ? error.message : String(error), "write_failed", true))
    }

    await pending.promise
  }

  private async sendRelayUnsubscribe(subscriptionId: string, privateKey: Buffer): Promise<void> {
    const lane: KernelSocketLane = "event"
    const socket = await this.ensureWebSocket(lane)
    const requestId = randomUUID()

    const pending = this.pendingRequests.register<void>(requestId, lane)
    pending.setRelayPrivateKey(privateKey)

    try {
      const frame = buildRelayUnsubscribeFrame(requestId, subscriptionId, privateKey)
      socket.send(JSON.stringify(frame))
    } catch (error) {
      pending.reject(new LocalIpcError("write relay unsubscribe", error instanceof Error ? error.message : String(error), "write_failed", true))
    }

    await pending.promise
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

      const fail = (operation: string, error: unknown, code: string | null = null, retryable = false) => {
        if (settled) {
          return
        }
        settled = true
        this.setWebSocketConnectPromise(lane, null)
        reject(new LocalIpcError(operation, formatTransportError(error, this.socketPath), code, retryable))
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
            const closeMessage = reason.length > 0
              ? reason.toString("utf8")
              : `kernel websocket closed${code ? ` (${code})` : ""}`
            this.rejectPending(closeMessage, lane)
            this.setWebSocket(lane, null)
            this.setRelayDaemonPublicKey(lane, null)
            this.clearKernelHeartbeat(lane)
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
          fail("write relay connect frame", error, "write_failed", true)
        }
      })

      const handleConnectError = (error: unknown) => fail("connect kernel websocket", error, "connection_closed", true)
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
    const pending = this.pendingRequests.take(requestId)
    if (!pending) {
      return
    }

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
    this.pendingRequests.rejectMatching(message, lane)
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
    this.reconnectDelayMs = KERNEL_RECONNECT_BASE_DELAY_MS
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
    }, this.reconnectDelayWithJitter(delayMs))
    this.reconnectDelayMs = this.nextReconnectDelayMs(delayMs)
  }

  private reconnectDelayWithJitter(delayMs: number): number {
    const boundedDelayMs = Math.max(delayMs, 0)
    if (boundedDelayMs < KERNEL_RECONNECT_BASE_DELAY_MS || this.reconnectJitterMs === 0) {
      return boundedDelayMs
    }
    const jitterMs = Math.floor(clampRandom(this.reconnectRandom()) * this.reconnectJitterMs)
    return Math.min(boundedDelayMs + jitterMs, KERNEL_RECONNECT_MAX_DELAY_MS + this.reconnectJitterMs)
  }

  private nextReconnectDelayMs(delayMs: number): number {
    return Math.min(
      Math.max(delayMs * 2, KERNEL_RECONNECT_BASE_DELAY_MS),
      KERNEL_RECONNECT_MAX_DELAY_MS,
    )
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
          kernelSubscriptionScopeValue(subscription),
        )
      } else {
        await this.sendWebSocket<Record<string, unknown>>(
          buildKernelSubscriptionTransportRequest(subscription, this.lastReceivedEventId),
          "event",
        )
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

  private destroyWebSocket(lane: KernelSocketLane): void {
    const socket = this.getWebSocket(lane)
    this.setWebSocket(lane, null)
    this.setWebSocketConnectPromise(lane, null)
    this.setRelayDaemonPublicKey(lane, null)
    if (socket && socket.readyState !== WebSocket.CLOSED) {
      this.setSuppressNextCloseEvent(lane, true)
      socket.terminate()
    }
  }
}

function clampRandom(value: number): number {
  if (!Number.isFinite(value)) {
    return 0
  }
  return Math.min(Math.max(value, 0), 0.999999)
}
