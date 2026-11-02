import { parseFrame, type RelayCloseFrame } from "./kernel-direct-types.js"

/** Lanes of one identity share the authenticated channel, with socket-local routing. */
export class KernelDirectCarrier {
  private readonly lanes = new Set<KernelDirectLane>()
  private readonly requests = new Map<string, KernelDirectLane>()
  private readonly subscriptions = new Map<string, { lane: KernelDirectLane; publicKey: string }>()
  private lost = false

  constructor(readonly socket: WebSocket, readonly daemonPublicKey: string, private readonly onLost: (close?: RelayCloseFrame) => void) {
    socket.addEventListener("message", event => this.receive((event as MessageEvent).data))
    socket.addEventListener("activity", () => this.broadcast(() => new Event("activity")))
    socket.addEventListener("pong", () => this.broadcast(() => new Event("pong")))
    socket.addEventListener("error", () => this.broadcast(() => new Event("error")))
    socket.addEventListener("close", event => {
      this.noteLost()
      const close = event as CloseEvent
      for (const lane of [...this.lanes]) lane.finish(close.code, close.reason)
    }, { once: true })
  }

  get open(): boolean { return this.socket.readyState === 1 && !this.lost }

  openLane(signal?: AbortSignal): WebSocket {
    signal?.throwIfAborted()
    if (!this.open) throw new Error("direct carrier closed")
    const lane = new KernelDirectLane(this)
    this.lanes.add(lane)
    const abort = () => lane.close()
    signal?.addEventListener("abort", abort, { once: true })
    lane.addEventListener("close", () => signal?.removeEventListener("abort", abort), { once: true })
    return lane as unknown as WebSocket
  }

  send(lane: KernelDirectLane, data: string): void {
    const frame = parseFrame<{ kind: string; request_id?: string; subscription_id?: string; client_public_key?: string }>(data)
    if (!frame) throw new Error("invalid direct lane frame")
    const requestId = frame.request_id, subscriptionId = frame.subscription_id
    if (requestId && this.requests.has(requestId) && this.requests.get(requestId) !== lane) throw new Error("direct lane routing id collision")
    if (frame.kind === "client_subscribe") {
      if (!subscriptionId || !frame.client_public_key) throw new Error("invalid direct subscription")
      if (this.subscriptions.has(subscriptionId) && this.subscriptions.get(subscriptionId)!.lane !== lane) throw new Error("direct lane routing id collision")
    }
    if (frame.kind === "client_unsubscribe" && (!subscriptionId || this.subscriptions.get(subscriptionId)?.lane !== lane)) throw new Error("direct lane does not own subscription")
    if (requestId) this.requests.set(requestId, lane)
    if (frame.kind === "client_subscribe") this.subscriptions.set(subscriptionId!, { lane, publicKey: frame.client_public_key! })
    if (frame.kind === "client_unsubscribe") this.subscriptions.delete(subscriptionId!)
    try { this.socket.send(data) }
    catch (error) { if (requestId) this.requests.delete(requestId); throw error }
  }

  retire(lane: KernelDirectLane): void {
    this.lanes.delete(lane)
    for (const [id, owner] of this.requests) if (owner === lane) this.requests.delete(id)
    for (const [id, owner] of this.subscriptions) {
      if (owner.lane !== lane) continue
      this.subscriptions.delete(id)
      if (this.open) {
        try { this.socket.send(JSON.stringify({ kind: "client_unsubscribe", request_id: crypto.randomUUID(), subscription_id: id, client_public_key: owner.publicKey })) }
        catch { this.socket.close(1002, "direct subscription retirement failed") }
      }
    }
    if (this.lanes.size === 0 && this.socket.readyState < 2) this.socket.close()
  }

  private receive(data: unknown): void {
    const frame = parseFrame<{ kind: string; request_id?: string; subscription_id?: string }>(data)
    if (!frame) { this.socket.close(1002, "invalid direct carrier frame"); return }
    if (frame.kind === "client_response" && frame.request_id) {
      const lane = this.requests.get(frame.request_id)
      this.requests.delete(frame.request_id)
      lane?.dispatchEvent(new MessageEvent("message", { data }))
    } else if (frame.kind === "client_event" && frame.subscription_id) {
      this.subscriptions.get(frame.subscription_id)?.lane.dispatchEvent(new MessageEvent("message", { data }))
    } else if (frame.kind === "close" || frame.kind === "client_connected") {
      if (frame.kind === "close") this.noteLost(frame as RelayCloseFrame)
      this.broadcast(() => new MessageEvent("message", { data }))
    } else this.socket.close(1002, "invalid direct carrier frame")
  }

  private noteLost(close?: RelayCloseFrame): void {
    if (this.lost || this.lanes.size === 0) return
    this.lost = true
    this.onLost(close)
  }

  private broadcast(event: () => Event): void {
    for (const lane of [...this.lanes]) lane.dispatchEvent(event())
  }
}

class KernelDirectLane extends EventTarget {
  readonly CONNECTING = 0
  readonly OPEN = 1
  readonly CLOSING = 2
  readonly CLOSED = 3
  readonly url = "webrtc:"
  readonly protocol = ""
  readonly extensions = ""
  binaryType: BinaryType = "arraybuffer"
  readyState = 1
  constructor(private readonly carrier: KernelDirectCarrier) { super() }
  get bufferedAmount(): number { return this.carrier.socket.bufferedAmount }
  send(data: string): void {
    if (this.readyState !== 1) throw new Error("direct lane closed")
    this.carrier.send(this, data)
  }
  ping(): void { (this.carrier.socket as WebSocket & { ping(): void }).ping() }
  close(code = 1000, reason = ""): void { this.finish(code, reason) }
  finish(code: number, reason: string): void {
    if (this.readyState === 3) return
    this.readyState = 3
    this.carrier.retire(this)
    const event = Object.assign(new Event("close"), { code, reason, wasClean: code === 1000 })
    this.dispatchEvent(event)
  }
}
