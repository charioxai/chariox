// MP-08/MP-11: Node's relay adapter; carrier selection and proofs are shared.
import WebSocket from "ws"
import type { EncryptedRelayPayload, RelayTarget } from "./kernel-transport-frames.js"
import type { RelayClientIdentity } from "./relay-crypto.js"
import { buildRelayConnectFrame } from "./relay-transport.js"
export class TerminalDirectAuthorization {
  private closed = false
  private readonly pending = new Set<WebSocket>()
  constructor(private readonly input: { relayUrl: string; token(): string; target: RelayTarget; identity: RelayClientIdentity }) {}
  close(): void { this.closed = true; for (const socket of this.pending) socket.terminate(); this.pending.clear() }
  authorize(bodies: readonly unknown[], expectedKey: string, signal?: AbortSignal): Promise<Record<string, unknown>[]> {
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
    }, socket => socket.send(JSON.stringify(buildRelayConnectFrame(this.input.token(), target))), signal)
  }

  private exchange<T>(url: string, options: WebSocket.ClientOptions,
    frame: (socket: WebSocket, body: Record<string, unknown>, resolve: (result: T) => void, reject: (error: Error) => void) => void,
    onOpen?: (socket: WebSocket) => void, signal?: AbortSignal): Promise<T> {
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
      const onAbort = () => finish(Error("terminal authorization cancelled"))
      const opened = () => { if (!settled) { try { onOpen?.(socket) } catch { onError() } } }
      let settled = false
      const finish = (error: Error | null, result?: T) => {
        if (settled) return
        settled = true; clearTimeout(timer); this.pending.delete(socket)
        signal?.removeEventListener("abort", onAbort); socket.off("open", opened)
        socket.off("message", onMessage); socket.off("error", onError); socket.off("close", onClose)
        // Prevent a late error between retiring a failed upgrade and close.
        socket.on("error", () => {})
        if (error || result !== socket) socket.terminate()
        if (error) reject(error); else resolve(result!)
      }
      socket.on("message", onMessage); socket.once("error", onError); socket.once("close", onClose)
      if (onOpen) socket.once("open", opened)
      signal?.addEventListener("abort", onAbort, { once: true })
      if (signal?.aborted) onAbort()
    })
  }
}
