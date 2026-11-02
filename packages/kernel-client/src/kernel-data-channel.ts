/*
 * MP-08/MP-11 protocol 917: the direct carrier over a WebRTC data channel.
 * The offer and answer travel inside the encrypted relay request that mints
 * the grant, so the DTLS fingerprints securing the channel were exchanged end
 * to end. The channel then carries the same grant/key handshake and encrypted
 * relay client frames as the WebSocket carrier did; this class gives it the
 * WebSocket surface the lane code already speaks.
 *
 * Wire: every message is binary, one kind byte then payload. Text frames
 * longer than one fragment span several MORE messages and a LAST.
 */

import { relayMaxMessageBytes } from "./kernel-direct-types.js"

const frameLast = 0
const frameMore = 1
const framePing = 2
const framePong = 3
const fragmentBytes = 16 * 1024
/** The kernel sends a response as one text frame: the relay's largest message, base64-wrapped in JSON. */
const maxMessageBytes = Math.ceil(relayMaxMessageBytes / 3) * 4 + 64 * 1024
const gatherTimeoutMs = 1_500
/**
 * A symmetric NAT never connects: the lane gives way to the relay well inside
 * 5 s (TR-15). The shaped hosted path (60 ms RTT) connects in under 1.5 s. The
 * budget covers the whole attempt (lease, ICE gathering, grant, connect), and
 * the channel itself gets as long to connect once it exists.
 */
export const directAttemptBudgetMs = 3_000
/** ICE "disconnected" this long means the path is gone; the lane falls back. */
const disconnectedGraceMs = 2_500
/**
 * The kernel sends at least a keepalive every second, so this much silence
 * means the path is gone (UDP dropped, network changed) well before ICE
 * consent gives up; the lane falls back to the relay (TR-16).
 */
const silenceLimitMs = 2_500

export type DataChannelOffer = { readonly peer: RTCPeerConnection; readonly channel: RTCDataChannel; readonly sdp: string }

/** Every Chariox relay answers STUN Binding requests on this UDP port. */
const relayStunPort = 3478

/**
 * The STUN responders on the relay hosts this tab uses. A browser behind NAT
 * learns its public address from them, so a kernel behind another NAT can
 * reach it.
 */
export function relayStunServers(relayUrls: readonly string[]): RTCIceServer[] {
  const hosts = new Set<string>()
  for (const relayUrl of relayUrls) {
    try { hosts.add(new URL(relayUrl).hostname) } catch { /* not a URL: no STUN host */ }
  }
  return [...hosts].filter(Boolean).map(host => ({ urls: `stun:${host}:${relayStunPort}` }))
}

export function dataChannelSupported(): boolean {
  return typeof globalThis.RTCPeerConnection === "function"
}

/** Creates the peer and its one ordered channel; resolves once host candidates are in the offer. */
export async function createDataChannelOffer(iceServers: readonly RTCIceServer[] = [], Peer: typeof RTCPeerConnection = globalThis.RTCPeerConnection): Promise<DataChannelOffer> {
  const peer = new Peer({ iceServers: [...iceServers] })
  const channel = peer.createDataChannel("chariox", { ordered: true })
  channel.binaryType = "arraybuffer"
  try {
    const describing = peer.setLocalDescription(await peer.createOffer())
    await new Promise<void>((resolve, reject) => {
      if (peer.iceGatheringState === "complete") { resolve(); return }
      const timer = setTimeout(done, gatherTimeoutMs)
      function done() { clearTimeout(timer); peer.removeEventListener("icegatheringstatechange", changed); resolve() }
      function changed() { if (peer.iceGatheringState === "complete") done() }
      peer.addEventListener("icegatheringstatechange", changed)
      void describing.then(() => { if (peer.iceGatheringState === "complete") done() }, error => { clearTimeout(timer); peer.removeEventListener("icegatheringstatechange", changed); reject(error) })
    })
    const sdp = peer.localDescription?.sdp
    if (!sdp) throw new Error("browser produced no WebRTC offer")
    return { peer, channel, sdp }
  } catch (error) {
    peer.close()
    throw error
  }
}

/** A data channel behind the WebSocket members the lane code uses. */
export class DataChannelSocket extends EventTarget {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSING = 2
  static readonly CLOSED = 3
  readonly CONNECTING = 0
  readonly OPEN = 1
  readonly CLOSING = 2
  readonly CLOSED = 3
  readonly url = "webrtc:"
  readonly protocol = ""
  readonly extensions = ""
  binaryType: BinaryType = "arraybuffer"
  readyState = DataChannelSocket.CONNECTING
  private pendingText: string | null = null
  private parts: Uint8Array[] = []
  private partBytes = 0
  private connectTimer: ReturnType<typeof setTimeout> | undefined
  private graceTimer: ReturnType<typeof setTimeout> | undefined
  private silenceTimer: ReturnType<typeof setTimeout> | undefined
  private lastReceivedMs = 0
  private readonly encoder = new TextEncoder()
  private readonly decoder = new TextDecoder("utf-8", { fatal: true })

  constructor(private readonly peer: RTCPeerConnection, private readonly channel: RTCDataChannel) {
    super()
    channel.binaryType = "arraybuffer"
    const opened = () => {
      clearTimeout(this.connectTimer)
      if (this.readyState !== DataChannelSocket.CONNECTING) return
      this.readyState = DataChannelSocket.OPEN
      this.lastReceivedMs = Date.now()
      this.watchSilence(silenceLimitMs)
      this.dispatchEvent(new Event("open"))
      if (this.pendingText !== null) {
        const text = this.pendingText; this.pendingText = null
        this.dispatchEvent(new MessageEvent("message", { data: text }))
      }
    }
    channel.addEventListener("open", opened)
    channel.addEventListener("message", (event) => {
      if (channel.readyState === "open") opened()
      this.lastReceivedMs = Date.now(); this.receive(event.data)
    })
    channel.addEventListener("close", () => this.finish(1006, "direct data channel closed"))
    channel.addEventListener("error", () => this.fail("direct data channel failed"))
    peer.addEventListener("connectionstatechange", () => {
      const state = peer.connectionState
      if (state === "failed" || state === "closed") this.fail(`direct path ${state}`)
      else if (state === "disconnected") {
        this.graceTimer ??= setTimeout(() => this.fail("direct path disconnected"), disconnectedGraceMs)
      } else if (state === "connected") { clearTimeout(this.graceTimer); this.graceTimer = undefined }
    })
    this.connectTimer = setTimeout(() => this.fail("direct path did not connect"), directAttemptBudgetMs)
    if (channel.readyState === "open") queueMicrotask(opened)
  }

  /** The lane code only sends text frames. */
  send(data: string): void {
    if (this.readyState !== DataChannelSocket.OPEN) throw new DOMException("data channel is not open", "InvalidStateError")
    const bytes = this.encoder.encode(data)
    if (bytes.length === 0) { this.channel.send(new Uint8Array([frameLast])); return }
    for (let offset = 0; offset < bytes.length; offset += fragmentBytes) {
      const chunk = bytes.subarray(offset, offset + fragmentBytes)
      const fragment = new Uint8Array(chunk.length + 1)
      fragment[0] = offset + fragmentBytes < bytes.length ? frameMore : frameLast
      fragment.set(chunk, 1)
      this.channel.send(fragment)
    }
  }

  ping(): void {
    if (this.readyState !== DataChannelSocket.OPEN) throw new Error("data channel is not open")
    this.channel.send(new Uint8Array([framePing]))
  }

  get bufferedAmount(): number { return this.channel.bufferedAmount }

  close(code = 1000, reason = ""): void {
    this.finish(code, reason)
  }

  private receive(data: unknown): void {
    const bytes = data instanceof ArrayBuffer ? new Uint8Array(data)
      : ArrayBuffer.isView(data) ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength) : null
    if (!bytes || bytes.length === 0) { this.fail("direct carrier sent an invalid frame"); return }
    const kind = bytes[0]
    if (kind === framePing) { if (this.readyState === DataChannelSocket.OPEN) this.channel.send(new Uint8Array([framePong])); return }
    if (kind === framePong) { this.dispatchEvent(new Event("pong")); return }
    if (kind !== frameMore && kind !== frameLast) { this.fail("direct carrier sent an unknown frame"); return }
    this.parts.push(bytes.subarray(1))
    this.partBytes += bytes.length - 1
    if (this.partBytes > maxMessageBytes) { this.fail("direct carrier frame is too large"); return }
    if (kind === frameMore) return
    const whole = new Uint8Array(this.partBytes)
    let offset = 0
    for (const part of this.parts) { whole.set(part, offset); offset += part.length }
    this.parts = []
    this.partBytes = 0
    let text: string
    try { text = this.decoder.decode(whole) } catch { this.fail("direct carrier frame is not text"); return }
    // SCTP implementations can deliver the challenge before the DCEP open acknowledgement.
    // Preserve WebSocket ordering: no application message reaches the lane before open.
    if (this.readyState === DataChannelSocket.CONNECTING) {
      if (this.pendingText !== null) { this.fail("direct carrier sent messages before opening"); return }
      this.pendingText = text
    } else if (this.readyState === DataChannelSocket.OPEN) this.dispatchEvent(new MessageEvent("message", { data: text }))
  }

  /** One timer per silence window, re-armed for what is left of it; not one per message. */
  private watchSilence(delayMs: number): void {
    this.silenceTimer = setTimeout(() => {
      if (this.readyState !== DataChannelSocket.OPEN) return
      const silentMs = Date.now() - this.lastReceivedMs
      if (silentMs >= silenceLimitMs) this.fail("direct path silent")
      else this.watchSilence(silenceLimitMs - silentMs)
    }, delayMs)
  }

  private fail(reason: string): void {
    if (this.readyState === DataChannelSocket.CLOSED) return
    this.dispatchEvent(new Event("error"))
    this.finish(1006, reason)
  }

  private finish(code: number, reason: string): void {
    if (this.readyState === DataChannelSocket.CLOSED) return
    this.readyState = DataChannelSocket.CLOSED
    clearTimeout(this.connectTimer)
    clearTimeout(this.graceTimer)
    clearTimeout(this.silenceTimer)
    try { this.channel.close() } catch { /* already closed */ }
    this.peer.close()
    this.dispatchEvent(new DataChannelCloseEvent(code, reason))
  }
}

/** The close event the lane code reads (`code`, `reason`), as a WebSocket would dispatch it. */
class DataChannelCloseEvent extends Event {
  readonly wasClean: boolean
  constructor(readonly code: number, readonly reason: string) {
    super("close")
    this.wasClean = code === 1000
  }
}
