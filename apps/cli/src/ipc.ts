import net from "node:net"
import { randomUUID } from "node:crypto"

import WebSocket from "ws"

const IPC_TIMEOUT_MS = 30_000

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

type PendingRequest<TResponse> = {
  resolve: (value: TResponse) => void
  reject: (error: LocalIpcError) => void
  timeout: NodeJS.Timeout
}

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
    event: "session_snapshot"
    session: Record<string, unknown>
    provider_run: Record<string, unknown> | null
  }
  | {
    event: "session_unavailable"
    session_id: string
    message: string
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
}

export class LocalIpcClient {
  readonly socketPath: string
  private websocket: WebSocket | null = null
  private websocketConnectPromise: Promise<WebSocket> | null = null
  private pending = new Map<string, PendingRequest<unknown>>()
  private eventHandlers = new Set<(event: KernelEvent) => void>()
  private activeKernelSubscription: KernelSubscriptionState | null = null
  private reconnectTimeout: NodeJS.Timeout | null = null
  private reconnectDelayMs = 250
  private lastReceivedEventId: number | null = null
  private suppressNextCloseEvent = false

  constructor(endpoint: string) {
    this.socketPath = endpoint
  }

  supportsKernelEvents() {
    return isWebSocketEndpoint(this.socketPath)
  }

  send<TResponse>(request: unknown): Promise<TResponse> {
    if (this.supportsKernelEvents()) {
      return this.sendWebSocket(request)
    }
    return this.sendLocalSocket(request)
  }

  async subscribeToKernelEvents(sessionId: string, attachmentId: string): Promise<void> {
    if (!this.supportsKernelEvents()) {
      return
    }
    this.activeKernelSubscription = { sessionId, attachmentId }
    try {
      await this.sendWebSocket<Record<string, unknown>>({
        __kernel_transport: {
          type: "subscribe",
          session_id: sessionId,
          attachment_id: attachmentId,
          resume_from_event_id: this.lastReceivedEventId,
        },
      })
      this.clearReconnectState()
    } catch (error) {
      this.scheduleReconnect()
      throw error
    }
  }

  async unsubscribeFromKernelEvents(): Promise<void> {
    if (!this.supportsKernelEvents()) {
      return
    }
    this.activeKernelSubscription = null
    this.clearReconnectState()
    if (!this.websocket || this.websocket.readyState !== WebSocket.OPEN) {
      return
    }
    await this.sendWebSocket<Record<string, unknown>>({
      __kernel_transport: {
        type: "unsubscribe",
      },
    })
  }

  async restartKernelEventStream(): Promise<void> {
    if (!this.supportsKernelEvents() || !this.activeKernelSubscription) {
      return
    }
    this.clearReconnectState()
    if (this.websocket && this.websocket.readyState !== WebSocket.CLOSED) {
      this.suppressNextCloseEvent = true
      this.websocket.terminate()
      this.websocket = null
      this.websocketConnectPromise = null
    }
    this.scheduleReconnect(25)
  }

  onKernelEvent(handler: (event: KernelEvent) => void) {
    this.eventHandlers.add(handler)
    if (this.supportsKernelEvents()) {
      void this.ensureWebSocket().catch(() => {
        // The request path will report the connection error with better context.
      })
    }
    return () => {
      this.eventHandlers.delete(handler)
    }
  }

  async close(): Promise<void> {
    this.activeKernelSubscription = null
    this.clearReconnectState()
    const socket = this.websocket
    this.websocket = null
    this.websocketConnectPromise = null
    if (!socket) {
      return
    }
    if (socket.readyState === WebSocket.CLOSED) {
      return
    }

    await new Promise<void>((resolve) => {
      this.suppressNextCloseEvent = true
      socket.once("close", () => resolve())
      socket.close()
    })
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

  private async sendWebSocket<TResponse>(request: unknown): Promise<TResponse> {
    const socket = await this.ensureWebSocket()
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
      })

      try {
        socket.send(JSON.stringify(normalizeWebSocketRequest(requestId, request)))
      } catch (error) {
        clearTimeout(timeout)
        this.pending.delete(requestId)
        reject(new LocalIpcError("write kernel request", error instanceof Error ? error.message : String(error), "write_failed", true))
      }
    })
  }

  private async ensureWebSocket(): Promise<WebSocket> {
    if (this.websocket?.readyState === WebSocket.OPEN) {
      return this.websocket
    }
    if (this.websocketConnectPromise) {
      return this.websocketConnectPromise
    }

    this.websocketConnectPromise = new Promise<WebSocket>((resolve, reject) => {
      const socket = new WebSocket(this.socketPath)
      let settled = false

      const fail = (operation: string, error: unknown) => {
        if (settled) {
          return
        }
        settled = true
        this.websocketConnectPromise = null
        reject(new LocalIpcError(operation, error instanceof Error ? error.message : String(error)))
      }

      socket.once("open", () => {
        settled = true
        this.websocket = socket
        this.websocketConnectPromise = null
        this.suppressNextCloseEvent = false
        socket.on("message", (data: WebSocket.RawData) => {
          this.handleWebSocketMessage(data)
        })
        socket.once("close", () => {
          const suppressed = this.suppressNextCloseEvent
          this.suppressNextCloseEvent = false
          this.rejectPending("kernel websocket closed")
          this.websocket = null
          if (!suppressed) {
            this.emitSyntheticEvent({
              event: "transport_closed",
              message: "kernel websocket closed",
            })
            this.scheduleReconnect()
          }
        })
        socket.once("error", (error: Error) => {
          const suppressed = this.suppressNextCloseEvent
          this.suppressNextCloseEvent = false
          this.rejectPending(error.message)
          this.websocket = null
          if (!suppressed) {
            this.emitSyntheticEvent({
              event: "transport_closed",
              message: error.message,
            })
            this.scheduleReconnect()
          }
        })
        resolve(socket)
      })

      socket.once("error", (error: Error) => fail("connect kernel websocket", error))
    })

    return this.websocketConnectPromise
  }

  private handleWebSocketMessage(data: WebSocket.RawData) {
    let frame: KernelTransportResponseFrame<unknown> | KernelTransportEventFrame<KernelEvent>
    try {
      frame = JSON.parse(String(data)) as KernelTransportResponseFrame<unknown> | KernelTransportEventFrame<KernelEvent>
    } catch (error) {
      this.rejectPending(error instanceof Error ? error.message : String(error))
      return
    }

    if (frame.type === "event") {
      this.lastReceivedEventId = frame.event_id
      for (const handler of this.eventHandlers) {
        handler(frame.event)
      }
      return
    }

    const pending = this.pending.get(frame.request_id)
    if (!pending) {
      return
    }
    clearTimeout(pending.timeout)
    this.pending.delete(frame.request_id)

    if (frame.error) {
      pending.reject(new LocalIpcError("handle kernel response", frame.error.message, frame.error.code, frame.error.retryable))
      return
    }
    if (frame.response == null) {
      pending.reject(new LocalIpcError("handle kernel response", "response envelope was empty"))
      return
    }

    pending.resolve(frame.response)
  }

  private rejectPending(message: string) {
    const pendingEntries = Array.from(this.pending.values())
    this.pending.clear()
    for (const pending of pendingEntries) {
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
      await this.sendWebSocket<Record<string, unknown>>({
        __kernel_transport: {
          type: "subscribe",
          session_id: subscription.sessionId,
          attachment_id: subscription.attachmentId,
          resume_from_event_id: this.lastReceivedEventId,
        },
      })
      this.clearReconnectState()
      this.emitSyntheticEvent({
        event: "transport_resumed",
        session_id: subscription.sessionId,
        resumed_from_event_id: this.lastReceivedEventId,
      })
    } catch {
      this.scheduleReconnect()
    }
  }
}

function isWebSocketEndpoint(value: string) {
  return value.startsWith("ws://") || value.startsWith("wss://")
}

function normalizeWebSocketRequest(requestId: string, request: unknown) {
  const transportRequest = extractTransportRequest(request)
  if (transportRequest?.type === "subscribe") {
    return {
      type: "subscribe",
      request_id: requestId,
      session_id: transportRequest.session_id,
      attachment_id: transportRequest.attachment_id,
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
  | { type: "subscribe"; session_id: string; attachment_id: string; resume_from_event_id?: number | null }
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
      resume_from_event_id:
        typeof transport.resume_from_event_id === "number" ? transport.resume_from_event_id : null,
    }
  }
  if (transport.type === "unsubscribe") {
    return { type: "unsubscribe" }
  }
  return null
}
