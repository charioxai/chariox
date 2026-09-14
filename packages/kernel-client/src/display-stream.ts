import {
  createRelayKeypair,
  decryptRelayPayload,
  encryptRelayPayload,
  type EncryptedRelayPayload,
  type RelayKeypair,
} from "./browser-relay-crypto.js"
import { getSliceDisplayEndpointRequest } from "./ipc-slice-requests.js"
import type { SliceDisplayEndpoint } from "./kernel-types-cloud.js"

const DISPLAY_WIRE_PROTOCOL = "chariox-display-v1"
const DISPLAY_FRAGMENT_BYTES = 64 * 1024
const DISPLAY_MESSAGE_BYTES = 4 * 1024 * 1024
const DISPLAY_ENCRYPTED_PACKET_BYTES = 128 * 1024
const DISPLAY_CONNECT_TIMEOUT_MS = 10_000
const DISPLAY_CLOSE_TIMEOUT_MS = 1_000

export type DisplayMessage = {
  readonly kind: "text" | "binary"
  readonly data: Uint8Array
}

export type SelkiesViewerControl = "START_VIDEO" | "STOP_VIDEO" | "REQUEST_KEYFRAME" | `CLIENT_FRAME_ACK ${number}`

export type DisplayKernelClient = {
  send<TResponse>(request: unknown): Promise<TResponse>
  close?: () => Promise<void> | void
}

export type DisplayWebSocket = {
  readonly readyState: number
  send(data: Uint8Array | ArrayBuffer): void
  close(code?: number, reason?: string): void
  addEventListener?: (type: string, listener: (event: unknown) => void) => void
  removeEventListener?: (type: string, listener: (event: unknown) => void) => void
  on?: (type: string, listener: (...args: unknown[]) => void) => void
  off?: (type: string, listener: (...args: unknown[]) => void) => void
  removeListener?: (type: string, listener: (...args: unknown[]) => void) => void
}

export type DisplayWebSocketConstructor = new (url: string) => DisplayWebSocket

export type SelkiesDisplayStream = {
  readonly endpoint: SliceDisplayEndpoint
  readonly viewerPublicKey: string
  sendControl(control: SelkiesViewerControl, options?: { signal?: AbortSignal }): Promise<void>
  receive(options?: { signal?: AbortSignal; timeoutMs?: number }): Promise<DisplayMessage>
  close(): Promise<void>
}

export type OpenSelkiesDisplayStreamOptions = {
  readonly client: DisplayKernelClient
  readonly sliceId: string
  readonly sessionId: string
  readonly attachmentId: string
  readonly webSocket?: DisplayWebSocketConstructor
  readonly signal?: AbortSignal
  readonly connectTimeoutMs?: number
}

/**
 * Authorize one Room-scoped Selkies endpoint through the public home-kernel
 * request, then open its encrypted one-time WebSocket. The home kernel remains
 * the admission and Room authority; this client only handles the resulting
 * display wire and never reads worker Room state.
 */
export async function openSelkiesDisplayStream(
  options: OpenSelkiesDisplayStreamOptions,
): Promise<SelkiesDisplayStream> {
  requireText(options.sliceId, "sliceId")
  requireText(options.sessionId, "sessionId")
  requireText(options.attachmentId, "attachmentId")
  if (!options.client || typeof options.client.send !== "function") {
    throw displayError("requires a public kernel client")
  }
  throwIfAborted(options.signal, "before display authorization")

  const viewer = await createRelayKeypair()
  const response = await awaitWithAbort(
    Promise.resolve().then(() => options.client.send(
      getSliceDisplayEndpointRequest(options.sliceId, {
        sessionId: options.sessionId,
        attachmentId: options.attachmentId,
        viewerPublicKey: viewer.publicKeyBase64,
      }),
    )),
    options.signal,
    () => closeClientAfterAbort(options.client),
    "display authorization",
  )
  const endpoint = validateSelkiesEndpoint(response, options.sliceId)
  const socket = await connectDisplaySocket(
    endpoint.url,
    options.webSocket ?? defaultWebSocketConstructor(),
    options.signal,
    options.connectTimeoutMs ?? DISPLAY_CONNECT_TIMEOUT_MS,
  )
  return new EncryptedSelkiesDisplayStream(socket, endpoint, viewer, options.signal)
}

class EncryptedSelkiesDisplayStream implements SelkiesDisplayStream {
  readonly endpoint: SliceDisplayEndpoint
  readonly viewerPublicKey: string

  private readonly socket: DisplayWebSocket
  private readonly viewer: RelayKeypair
  private readonly peerPublicKey: string
  private readonly streamId: string
  private readonly removeSocketListeners: () => void
  private readonly queue: DisplayMessage[] = []
  private readonly waiters: Array<{
    resolve: (message: DisplayMessage) => void
    reject: (error: unknown) => void
    removeAbort?: () => void
    timer?: ReturnType<typeof setTimeout>
  }> = []
  private sendSequence = 0
  private receiveSequence = 0
  private partialKind: "text" | "binary" | null = null
  private partial = new Uint8Array()
  private failure: Error | null = null
  private closed = false

  constructor(
    socket: DisplayWebSocket,
    endpoint: SliceDisplayEndpoint,
    viewer: RelayKeypair,
    signal?: AbortSignal,
  ) {
    this.socket = socket
    this.endpoint = endpoint
    this.viewer = viewer
    this.viewerPublicKey = viewer.publicKeyBase64
    this.peerPublicKey = requireText(endpoint.peer_public_key, "endpoint.peer_public_key")
    this.streamId = requireText(endpoint.stream_id, "endpoint.stream_id")
    const onMessage = (...args: unknown[]) => {
      void this.acceptSocketMessage(args).catch((error: unknown) => this.fail(error))
    }
    const onError = () => this.fail(displayError("WebSocket reported a transport error"))
    const onClose = () => {
      if (!this.closed) this.fail(displayError("WebSocket closed before the stream was closed"))
    }
    const removeMessage = addSocketListener(socket, "message", onMessage)
    const removeError = addSocketListener(socket, "error", onError)
    const removeClose = addSocketListener(socket, "close", onClose)
    this.removeSocketListeners = () => {
      removeMessage()
      removeError()
      removeClose()
    }
    if (signal) {
      const abort = () => this.fail(abortError("display stream"))
      signal.addEventListener("abort", abort, { once: true })
      const previousRemove = this.removeSocketListeners
      this.removeSocketListeners = () => {
        signal.removeEventListener("abort", abort)
        previousRemove()
      }
      if (signal.aborted) abort()
    }
  }

  async sendControl(control: SelkiesViewerControl, options: { signal?: AbortSignal } = {}): Promise<void> {
    if (!isSafeViewerControl(control)) {
      throw displayError("rejected an unsafe viewer control")
    }
    this.ensureOpen()
    await this.sendMessage("text", new TextEncoder().encode(control), options.signal)
  }

  receive(options: { signal?: AbortSignal; timeoutMs?: number } = {}): Promise<DisplayMessage> {
    if (this.queue.length > 0) return Promise.resolve(this.queue.shift()!)
    if (this.failure) return Promise.reject(this.failure)
    if (this.closed) return Promise.reject(displayError("is closed"))
    throwIfAborted(options.signal, "before receiving display data")
    return new Promise<DisplayMessage>((resolve, reject) => {
      const waiter = { resolve, reject } as {
        resolve: (message: DisplayMessage) => void
        reject: (error: unknown) => void
        removeAbort?: () => void
        timer?: ReturnType<typeof setTimeout>
      }
      const abort = () => {
        this.removeWaiter(waiter)
        this.fail(abortError("display receive"))
      }
      if (options.signal) {
        options.signal.addEventListener("abort", abort, { once: true })
        waiter.removeAbort = () => options.signal?.removeEventListener("abort", abort)
      }
      if (options.timeoutMs !== undefined) {
        if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) {
          this.removeWaiter(waiter)
          reject(displayError("receive timeout must be positive"))
          return
        }
        waiter.timer = setTimeout(() => {
          this.removeWaiter(waiter)
          this.fail(displayError("did not receive display data before the deadline"))
        }, options.timeoutMs)
      }
      this.waiters.push(waiter)
    })
  }

  async close(): Promise<void> {
    if (this.closed) return
    this.closed = true
    this.removeSocketListeners()
    const waitForClose = waitForSocketClose(this.socket)
    try {
      if (this.socket.readyState !== 3) this.socket.close()
    } catch {
      // The caller is already closing a display stream; a failed close is terminal.
    }
    await waitForClose
    this.rejectWaiters(displayError("was closed"))
  }

  private ensureOpen() {
    if (this.failure) throw this.failure
    if (this.closed) throw displayError("is closed")
    if (this.socket.readyState !== 1) throw displayError("WebSocket is not open")
  }

  private async sendMessage(kind: "text" | "binary", data: Uint8Array, signal?: AbortSignal): Promise<void> {
    const count = Math.max(Math.ceil(data.byteLength / DISPLAY_FRAGMENT_BYTES), 1)
    const nextSequence = this.sendSequence + count
    if (!Number.isSafeInteger(nextSequence)) throw displayError("send sequence is exhausted")
    for (let index = 0; index < count; index += 1) {
      throwIfAborted(signal, "before sending display data")
      const start = index * DISPLAY_FRAGMENT_BYTES
      const end = Math.min(start + DISPLAY_FRAGMENT_BYTES, data.byteLength)
      const fragment = {
        protocol: DISPLAY_WIRE_PROTOCOL,
        stream_id: this.streamId,
        sender: "viewer",
        sequence: this.sendSequence + index,
        kind,
        final_fragment: index + 1 === count,
        data_base64: bytesToBase64(data.subarray(start, end)),
      }
      const encrypted = await awaitWithAbort(
        encryptRelayPayload(this.peerPublicKey, JSON.stringify(fragment), this.viewer),
        signal,
        () => closeSocketNow(this.socket),
        "display send",
      )
      this.ensureOpen()
      this.socket.send(encodedPayload(encrypted.payload))
    }
    this.sendSequence = nextSequence
  }

  private async acceptSocketMessage(args: unknown[]): Promise<void> {
    if (this.closed) return
    if (args.length > 1 && args[1] !== true) {
      throw displayError("received a non-binary Selkies packet")
    }
    const raw = socketMessageValue(args)
    const bytes = await socketMessageBytes(raw)
    if (bytes.byteLength > DISPLAY_ENCRYPTED_PACKET_BYTES * 2) {
      throw displayError("received an oversized Selkies packet")
    }
    const payload = parseEncryptedPayload(bytes)
    if (JSON.stringify(payload).length > DISPLAY_ENCRYPTED_PACKET_BYTES) {
      throw displayError("received an oversized encrypted Selkies packet")
    }
    const plaintext = await decryptRelayPayload(this.viewer.privateKey, payload, this.peerPublicKey)
    const fragment = parseDisplayFragment(plaintext)
    if (
      fragment.protocol !== DISPLAY_WIRE_PROTOCOL
      || fragment.stream_id !== this.streamId
      || fragment.sender !== "kernel"
      || fragment.sequence !== this.receiveSequence
    ) {
      throw displayError("received a display stream, direction, or sequence mismatch")
    }
    const data = base64ToBytes(fragment.data_base64)
    if (
      data.byteLength > DISPLAY_FRAGMENT_BYTES
      || this.partial.byteLength + data.byteLength > DISPLAY_MESSAGE_BYTES
      || (!fragment.final_fragment && data.byteLength === 0)
      || (this.partialKind !== null && this.partialKind !== fragment.kind)
    ) {
      throw displayError("received invalid or oversized display fragments")
    }
    this.receiveSequence += 1
    this.partialKind = fragment.kind
    this.partial = concatBytes(this.partial, data)
    if (!fragment.final_fragment) return
    const message = { kind: fragment.kind, data: this.partial }
    this.partial = new Uint8Array()
    this.partialKind = null
    if (fragment.kind === "text") {
      try {
        new TextDecoder("utf-8", { fatal: true }).decode(message.data)
      } catch {
        throw displayError("received invalid UTF-8 display text")
      }
    }
    const waiter = this.waiters.shift()
    if (waiter) {
      clearTimeout(waiter.timer)
      waiter.removeAbort?.()
      waiter.resolve(message)
    } else {
      this.queue.push(message)
    }
  }

  private fail(error: unknown) {
    if (this.failure || this.closed) return
    this.failure = error instanceof Error ? error : displayError(String(error))
    this.removeSocketListeners()
    this.rejectWaiters(this.failure)
    closeSocketNow(this.socket)
  }

  private rejectWaiters(error: Error) {
    for (const waiter of this.waiters.splice(0)) {
      clearTimeout(waiter.timer)
      waiter.removeAbort?.()
      waiter.reject(error)
    }
  }

  private removeWaiter(waiter: { reject: (error: unknown) => void; removeAbort?: () => void; timer?: ReturnType<typeof setTimeout> }) {
    const index = this.waiters.indexOf(waiter as never)
    if (index >= 0) this.waiters.splice(index, 1)
    clearTimeout(waiter.timer)
    waiter.removeAbort?.()
  }
}

function validateSelkiesEndpoint(response: unknown, sliceId: string): SliceDisplayEndpoint {
  if (!isRecord(response) || !isRecord(response.SliceDisplayEndpoint)) {
    throw displayError("authorization returned an unexpected response")
  }
  const endpoint = response.SliceDisplayEndpoint.endpoint
  if (!isRecord(endpoint)) throw displayError("authorization returned no endpoint")
  if (endpoint.slice_id !== sliceId || endpoint.kind !== "selkies" || endpoint.access !== "tunnel") {
    throw displayError("authorization returned the wrong Selkies endpoint")
  }
  const url = requireText(endpoint.url, "endpoint.url")
  let parsed: URL
  try {
    parsed = new URL(url)
  } catch {
    throw displayError("authorization returned an invalid WebSocket URL")
  }
  if (parsed.protocol !== "ws:" && parsed.protocol !== "wss:") {
    throw displayError("authorization returned a non-WebSocket URL")
  }
  if (endpoint.stream_protocol !== DISPLAY_WIRE_PROTOCOL) {
    throw displayError("authorization returned an unsupported display protocol")
  }
  requireText(endpoint.stream_id, "endpoint.stream_id")
  requireText(endpoint.peer_public_key, "endpoint.peer_public_key")
  return endpoint as unknown as SliceDisplayEndpoint
}

async function connectDisplaySocket(
  url: string,
  Constructor: DisplayWebSocketConstructor,
  signal: AbortSignal | undefined,
  timeoutMs: number,
): Promise<DisplayWebSocket> {
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0) throw displayError("WebSocket deadline must be positive")
  throwIfAborted(signal, "before opening display WebSocket")
  let socket: DisplayWebSocket
  try {
    socket = new Constructor(url)
  } catch (error) {
    throw displayError(`could not open the display WebSocket: ${errorMessage(error)}`)
  }
  return new Promise<DisplayWebSocket>((resolve, reject) => {
    let settled = false
    let timer: ReturnType<typeof setTimeout> | undefined
    const cleanup: Array<() => void> = []
    const finish = (callback: () => void) => {
      if (settled) return
      settled = true
      if (timer) clearTimeout(timer)
      signal?.removeEventListener("abort", onAbort)
      for (const remove of cleanup.splice(0)) remove()
      callback()
    }
    const onOpen = () => finish(() => resolve(socket))
    const onError = (error: unknown) => finish(() => reject(displayError(`display WebSocket failed: ${errorMessage(error)}`)))
    const onClose = () => finish(() => reject(displayError("display WebSocket closed before opening")))
    const onAbort = () => {
      closeSocketNow(socket)
      finish(() => reject(abortError("display WebSocket")))
    }
    cleanup.push(addSocketListener(socket, "open", onOpen))
    cleanup.push(addSocketListener(socket, "error", onError))
    cleanup.push(addSocketListener(socket, "close", onClose))
    timer = setTimeout(() => {
      closeSocketNow(socket)
      finish(() => reject(displayError("display WebSocket did not open before the deadline")))
    }, timeoutMs)
    signal?.addEventListener("abort", onAbort, { once: true })
    if (signal?.aborted) onAbort()
  })
}

function waitForSocketClose(socket: DisplayWebSocket): Promise<void> {
  if (socket.readyState === 3) return Promise.resolve()
  return new Promise((resolve) => {
    let timer: ReturnType<typeof setTimeout> | undefined
    const remove = addSocketListener(socket, "close", () => {
      if (timer) clearTimeout(timer)
      remove()
      resolve()
    })
    timer = setTimeout(() => {
      remove()
      resolve()
    }, DISPLAY_CLOSE_TIMEOUT_MS)
  })
}

function addSocketListener(socket: DisplayWebSocket, event: string, handler: (...args: unknown[]) => void): () => void {
  if (socket.addEventListener) {
    const listener = (value: unknown) => handler(value)
    socket.addEventListener(event, listener)
    return () => socket.removeEventListener?.(event, listener)
  }
  if (socket.on) {
    socket.on(event, handler)
    return () => {
      socket.off?.(event, handler)
      socket.removeListener?.(event, handler)
    }
  }
  throw displayError("WebSocket implementation has no event listener API")
}

function defaultWebSocketConstructor(): DisplayWebSocketConstructor {
  const Constructor = (globalThis as { WebSocket?: unknown }).WebSocket
  if (typeof Constructor !== "function") {
    throw displayError("requires a WebSocket implementation")
  }
  return Constructor as DisplayWebSocketConstructor
}

function socketMessageValue(args: unknown[]): unknown {
  const value = args[0]
  if (args.length === 1 && isRecord(value) && "data" in value) return value.data
  return value
}

async function socketMessageBytes(value: unknown): Promise<Uint8Array> {
  if (value instanceof Uint8Array) return new Uint8Array(value)
  if (value instanceof ArrayBuffer) return new Uint8Array(value)
  if (isRecord(value) && typeof value.arrayBuffer === "function") {
    const bytes = await value.arrayBuffer()
    if (bytes instanceof ArrayBuffer) return new Uint8Array(bytes)
  }
  throw displayError("received a non-binary WebSocket message")
}

function parseEncryptedPayload(bytes: Uint8Array): EncryptedRelayPayload {
  let value: unknown
  try {
    value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes))
  } catch {
    throw displayError("received malformed encrypted display payload")
  }
  if (!isRecord(value)
    || !hasExactKeys(value, ["ciphertext", "nonce", "sender_public_key"])
    || typeof value.sender_public_key !== "string"
    || typeof value.nonce !== "string"
    || typeof value.ciphertext !== "string") {
    throw displayError("received malformed encrypted display payload")
  }
  return {
    sender_public_key: value.sender_public_key,
    nonce: value.nonce,
    ciphertext: value.ciphertext,
  }
}

function parseDisplayFragment(plaintext: string): {
  protocol: string
  stream_id: string
  sender: "kernel" | "viewer"
  sequence: number
  kind: "text" | "binary"
  final_fragment: boolean
  data_base64: string
} {
  let value: unknown
  try {
    value = JSON.parse(plaintext)
  } catch {
    throw displayError("received malformed encrypted display fragment")
  }
  if (!isRecord(value)
    || !hasExactKeys(value, [
      "data_base64",
      "final_fragment",
      "kind",
      "protocol",
      "sender",
      "sequence",
      "stream_id",
    ])
    || typeof value.protocol !== "string"
    || typeof value.stream_id !== "string"
    || (value.sender !== "kernel" && value.sender !== "viewer")
    || typeof value.sequence !== "number"
    || !Number.isSafeInteger(value.sequence)
    || (value.kind !== "text" && value.kind !== "binary")
    || typeof value.final_fragment !== "boolean"
    || typeof value.data_base64 !== "string") {
    throw displayError("received malformed encrypted display fragment")
  }
  return value as {
    protocol: string
    stream_id: string
    sender: "kernel" | "viewer"
    sequence: number
    kind: "text" | "binary"
    final_fragment: boolean
    data_base64: string
  }
}

function isSafeViewerControl(value: string): value is SelkiesViewerControl {
  if (value === "START_VIDEO" || value === "STOP_VIDEO" || value === "REQUEST_KEYFRAME") return true
  const match = /^CLIENT_FRAME_ACK ([0-9]{1,5})$/.exec(value)
  return match !== null && Number(match[1]) <= 65_535
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = ""
  for (const byte of bytes) binary += String.fromCharCode(byte)
  return btoa(binary)
}

function base64ToBytes(value: string): Uint8Array {
  if (value.length % 4 !== 0 || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(value)) {
    throw displayError("received invalid display bytes")
  }
  let binary: string
  try {
    binary = atob(value)
  } catch {
    throw displayError("received invalid display bytes")
  }
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index)
  return bytes
}

function concatBytes(first: Uint8Array<ArrayBufferLike>, second: Uint8Array<ArrayBufferLike>): Uint8Array<ArrayBuffer> {
  const result = new Uint8Array(first.byteLength + second.byteLength)
  result.set(first)
  result.set(second, first.byteLength)
  return result
}

function encodedPayload(payload: EncryptedRelayPayload): Uint8Array {
  return new TextEncoder().encode(JSON.stringify(payload))
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}

function hasExactKeys(value: Record<string, unknown>, expected: string[]): boolean {
  const actual = Object.keys(value).sort()
  return actual.length === expected.length && actual.every((key, index) => key === expected.slice().sort()[index])
}

function requireText(value: unknown, label: string): string {
  if (typeof value !== "string" || value.trim() === "") throw displayError(`requires ${label}`)
  return value
}

function throwIfAborted(signal: AbortSignal | undefined, phase: string) {
  if (!signal?.aborted) return
  throw abortError(phase)
}

async function awaitWithAbort<T>(
  operation: Promise<T>,
  signal: AbortSignal | undefined,
  onAbort: () => void,
  phase: string,
): Promise<T> {
  if (!signal) return operation
  throwIfAborted(signal, `before ${phase}`)
  let rejectAbort!: (reason: unknown) => void
  const aborted = new Promise<never>((_, reject) => {
    rejectAbort = reject
  })
  const abort = () => {
    onAbort()
    rejectAbort(abortError(phase))
  }
  signal.addEventListener("abort", abort, { once: true })
  try {
    return await Promise.race([operation, aborted])
  } finally {
    signal.removeEventListener("abort", abort)
  }
}

function closeSocketNow(socket: DisplayWebSocket) {
  try {
    if (socket.readyState !== 3) socket.close()
  } catch {
    // The error already being reported is the useful operation result.
  }
}

function closeClientAfterAbort(client: DisplayKernelClient) {
  try {
    Promise.resolve(client.close?.()).catch(() => {})
  } catch {
    // Abort still rejects even when client teardown cannot complete.
  }
}

function abortError(phase: string): Error {
  const error = displayError(`was aborted ${phase}`)
  error.name = "AbortError"
  return error
}

function displayError(message: string): Error {
  return new Error(`Selkies display stream ${message}`)
}

function errorMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message
  if (typeof error === "string" && error.trim()) return error
  return "transport error"
}
