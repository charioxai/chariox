import { createHash, randomUUID } from "node:crypto"
import WebSocket from "ws"
import {createRelayKeypair, encryptRelayPayload, decryptRelayPayload, type EncryptedRelayPayload, type RelayKeypair} from "./browser-relay-crypto.js"
import { RelayFrameReceiver, relayTransportUrl } from "./relay-frame-transport.js"

export type AgentTriggerRelayBootstrap = { relayUrl: string; relayToken: string; relayTokenExpiresAt: string | Date; target: { daemonId?: string; kernelRef: string } }
type Pending = { resolve(value: unknown): void; reject(error: Error): void; timer: ReturnType<typeof setTimeout> }

class DeliveryUnconfirmed extends Error {
  readonly statusCode = 502
  constructor() { super("delivery unconfirmed; check agent history before sending again") }
}

function refusal(value: unknown): Error {
  const error = value && typeof value === "object" ? value as {code?: string; message?: string} : {}
  const reason = `${error.code ?? ""} ${error.message ?? ""}`
  const statusCode = /authorization_denied|unauthoriz|forbidden|permission|admission|not a member|cannot perform.*owned by/i.test(reason) ? 403
    : /not_found|was not found|no longer exists/i.test(reason) ? 404 : 409
  return Object.assign(Error(statusCode === 403 ? "agent access denied" : statusCode === 404 ? "trigger not found" : "agent request refused"), { statusCode })
}

/** A request is sent once. A lost reply must never replay an HTTP firing. */
export class AgentTriggerIngressClient {
  private readonly pending = new Map<string, Pending>()
  private peer: string | null = null
  private receive: Promise<void> = Promise.resolve()
  private closed = false
  private resolveConnected!: () => void
  private rejectConnected!: (error: Error) => void
  private readonly connected = new Promise<void>((resolve, reject) => { this.resolveConnected = resolve; this.rejectConnected = reject })

  private constructor(private readonly socket: WebSocket, private readonly keypair: RelayKeypair, private readonly kernelId: string) {
    const receiver = new RelayFrameReceiver()
    this.socket.on("message", data => {
      this.receive = this.receive.then(async () => {
        const decoded = receiver.receive(data.toString(), receipt => socket.send(receipt))
        if (decoded) await this.message(typeof decoded === "string" ? JSON.parse(decoded) : decoded)
      }).catch(() => { void this.close() })
    })
    this.socket.on("error", () => { void this.close() })
    this.socket.on("close", () => { void this.close() })
  }

  static async connect(kernelId: string, bootstrap: (thumbprint: string) => Promise<AgentTriggerRelayBootstrap>, origin: string): Promise<AgentTriggerIngressClient> {
    const keypair = await createRelayKeypair()
    const thumbprint = createHash("sha256").update(keypair.publicKeyBase64, "utf8").digest("hex")
    const grant = await bootstrap(thumbprint)
    if (!grant.relayUrl.startsWith("wss://") || grant.target.daemonId !== kernelId) throw Error("agent relay target unavailable")
    const expiresAt = new Date(grant.relayTokenExpiresAt).getTime()
    if (!Number.isFinite(expiresAt) || expiresAt <= Date.now()) throw Error("agent authorization expired")
    const socket = new WebSocket(relayTransportUrl(grant.relayUrl), { origin, maxPayload: 8 * 1024 * 1024, handshakeTimeout: 10_000 })
    const client = new AgentTriggerIngressClient(socket, keypair, kernelId)
    const timer = setTimeout(() => { void client.close() }, 10_000)
    socket.once("open", () => socket.send(JSON.stringify({ kind: "client_connect", auth_token: grant.relayToken, target: { daemon_id: kernelId } })))
    try {
      await client.connected
      return client
    } catch (error) { await client.close(); throw error }
    finally { clearTimeout(timer) }
  }

  async send<T = Record<string, unknown>>(request: unknown): Promise<T> {
    if (!request || typeof request !== "object" || Object.keys(request).length !== 1 || !("InvokeAgentTrigger" in request)) throw Error("Only HTTP trigger invocation is admitted")
    const id = randomUUID()
    if (!this.peer || this.closed) throw Error("agent offline")
    const encrypted_request = (await encryptRelayPayload(this.peer, JSON.stringify({ command_id: id, request }), this.keypair)).payload
    return this.request<T>(id, { kind: "client_request", request_id: id, target: { daemon_id: this.kernelId }, encrypted_request })
  }

  private request<T>(id: string, frame: unknown): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      if (this.closed) { reject(Error("agent offline")); return }
      const timer = setTimeout(() => { this.pending.delete(id); reject(new DeliveryUnconfirmed()) }, 15_000)
      this.pending.set(id, { resolve: value => resolve(value as T), reject, timer })
      this.socket.send(JSON.stringify(frame), error => {
        if (!error) return
        clearTimeout(timer); this.pending.delete(id); reject(new DeliveryUnconfirmed())
      })
    })
  }

  private async decrypt(payload: EncryptedRelayPayload): Promise<unknown> {
    if (payload.sender_public_key !== this.peer) throw Error("agent relay identity mismatch")
    return JSON.parse(await decryptRelayPayload(this.keypair.privateKey, payload))
  }

  private async message(frame: Record<string, unknown>): Promise<void> {
    if (this.closed) return
    if (frame.kind === "client_connected") {
      const target = frame.target as {daemon_id?: string} | undefined
      if (target?.daemon_id !== this.kernelId || typeof frame.daemon_public_key !== "string" || (this.peer && this.peer !== frame.daemon_public_key)) throw Error("agent relay identity mismatch")
      this.peer = frame.daemon_public_key
      this.resolveConnected()
    } else if (frame.kind === "client_response") {
      const pending = this.pending.get(String(frame.request_id))
      if (!pending) return
      this.pending.delete(String(frame.request_id)); clearTimeout(pending.timer)
      if (frame.error) { pending.reject(refusal(frame.error)); return }
      try {
        const value = frame.encrypted_response ? await this.decrypt(frame.encrypted_response as EncryptedRelayPayload) : null
        if (value && typeof value === "object" && "Error" in value) pending.reject(refusal(value.Error))
        else pending.resolve(value)
      } catch { pending.reject(new DeliveryUnconfirmed()) }
    } else if (frame.kind === "close") await this.close()
  }

  async close(): Promise<void> {
    if (this.closed) return
    this.closed = true
    this.rejectConnected(Error("agent offline"))
    for (const pending of this.pending.values()) { clearTimeout(pending.timer); pending.reject(new DeliveryUnconfirmed()) }
    this.pending.clear()
    this.socket.terminate()
  }
}
