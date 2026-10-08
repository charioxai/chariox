import type { EncryptedRelayPayload as RelayPayload } from "./kernel-transport-frames.js"

// Existing Cloud WebCrypto implementation, exposed for browser-side clients.
const relayNonceLength = 12
const relayInfo = new TextEncoder().encode("chariox-relay-v1")

export type EncryptedRelayPayload = Readonly<Omit<RelayPayload, "ciphertext"> & { ciphertext: string | Uint8Array }>

// MP-08/MP-10: browser websocket negotiation exposes the accepted protocol.
// A legacy relay cannot echo v96, so retry its existing connection contract.
export async function connectRelaySocket(url: string, binary = true): Promise<WebSocket> {
  const connect = (offer: boolean) => new Promise<WebSocket>((resolve, reject) => {
    const socket = offer ? new WebSocket(url, ["chariox-relay-binary-v96"]) : new WebSocket(url)
    socket.binaryType = "arraybuffer"
    const timer = setTimeout(() => { socket.close(); reject(new Error("relay handshake timeout")) }, 10000)
    socket.onopen = () => { clearTimeout(timer); resolve(socket) }
    socket.onerror = () => { clearTimeout(timer); socket.close(); reject(new Error("relay handshake failed")) }
  })
  if (binary) { try { return await connect(true) } catch { /* Legacy negotiation fallback. */ } }
  return connect(false)
}

export function decodeBinaryRelayEvent(bytes: Uint8Array): {
  kind: "client_event"; subscription_id: string; event_id: number; encrypted_event: EncryptedRelayPayload
} {
  if (bytes.length < 8 || bytes[0] !== 0x43 || bytes[1] !== 0x58 || bytes[2] !== 0x52 || bytes[3] !== 0x31) throw new Error("invalid binary relay event")
  const size = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(4)
  if (size < 2 || size > 4096 || bytes.length < 8 + size + 16 || bytes.length - 8 - size > 1024 * 1024) throw new Error("invalid binary relay event bounds")
  const header = JSON.parse(new TextDecoder().decode(bytes.subarray(8, 8 + size))) as Record<string, unknown>
  if (header.kind !== "client_event" || typeof header.subscription_id !== "string" || header.subscription_id.length < 1 || header.subscription_id.length > 1024
    || !Number.isSafeInteger(header.event_id) || (header.event_id as number) < 0
    || typeof header.sender_public_key !== "string" || header.sender_public_key.length < 1 || header.sender_public_key.length > 256
    || typeof header.nonce !== "string" || header.nonce.length !== 16) throw new Error("invalid binary relay event header")
  return { kind: "client_event", subscription_id: header.subscription_id, event_id: header.event_id as number,
    encrypted_event: { sender_public_key: header.sender_public_key, nonce: header.nonce, ciphertext: bytes.subarray(8 + size) } }
}

export type RelayKeypair = {
  readonly privateKey: CryptoKey
  readonly publicKeyBase64: string
}

export async function relayPublicKeyThumbprint(publicKeyBase64: string): Promise<string> {
  // Kernel terminal pairing hashes the encoded string, not decoded key bytes.
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(publicKeyBase64))
  return Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, "0")).join("")
}

export async function createRelayKeypair(): Promise<RelayKeypair> {
  const keypair = await crypto.subtle.generateKey(
    { name: "ECDH", namedCurve: "P-256" },
    false,
    ["deriveBits"],
  ) as CryptoKeyPair
  const publicKey = await crypto.subtle.exportKey("raw", keypair.publicKey)
  return {
    privateKey: keypair.privateKey,
    publicKeyBase64: bytesToBase64(new Uint8Array(publicKey)),
  }
}

export async function encryptRelayPayload(
  peerPublicKeyBase64: string,
  plaintext: string,
  sender?: RelayKeypair,
): Promise<{ readonly keypair: RelayKeypair; readonly payload: Readonly<RelayPayload> }> {
  // Paired clients retain their sender identity across consent/read requests.
  // Ordinary requests still receive a fresh ephemeral keypair.
  const keypair = sender ?? await createRelayKeypair()
  const key = await deriveRelayAesKey(keypair.privateKey, peerPublicKeyBase64)
  const nonce = crypto.getRandomValues(new Uint8Array(relayNonceLength))
  const ciphertext = await crypto.subtle.encrypt(
    { name: "AES-GCM", iv: bytesToArrayBuffer(nonce) },
    key,
    bytesToArrayBuffer(new TextEncoder().encode(plaintext)),
  )
  return {
    keypair,
    payload: {
      sender_public_key: keypair.publicKeyBase64,
      nonce: bytesToBase64(nonce),
      ciphertext: bytesToBase64(new Uint8Array(ciphertext)),
    },
  }
}

export async function decryptRelayPayload(
  privateKey: CryptoKey,
  payload: EncryptedRelayPayload,
  expectedSenderPublicKey?: string,
): Promise<string> {
  return new TextDecoder().decode(await decryptRelayBytes(privateKey, payload, expectedSenderPublicKey))
}

// MP-08/MP-10: protocol 466 display events are binary (`CXD1`, u32 header
// length, JSON header, raw payload); every other event stays JSON text.
export async function decryptRelayEvent(
  privateKey: CryptoKey,
  payload: EncryptedRelayPayload,
  expectedSenderPublicKey?: string,
): Promise<unknown> {
  const bytes = await decryptRelayBytes(privateKey, payload, expectedSenderPublicKey)
  return decodeDisplayEvent(bytes) ?? JSON.parse(new TextDecoder().decode(bytes))
}

// Returns null for non-display plaintext. Each `data:[offset,length]` becomes a
// view of the raw payload; segments must be contiguous and cover it exactly.
export function decodeDisplayEvent(bytes: Uint8Array): Record<string, unknown> | null {
  if (bytes.length < 8 || bytes[0] !== 0x43 || bytes[1] !== 0x58 || bytes[2] !== 0x44 || bytes[3] !== 0x31) return null
  const size = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(4)
  if (size < 2 || size > 64 * 1024 || 8 + size > bytes.length) throw new Error("invalid display event header")
  const event = JSON.parse(new TextDecoder().decode(bytes.subarray(8, 8 + size))) as Record<string, unknown>
  const frame = event.frame as Record<string, unknown> | undefined
  if (event.event !== "kernel_browser_frame" || typeof frame !== "object" || frame === null) throw new Error("invalid display event")
  const payload = bytes.subarray(8 + size)
  let cursor = 0
  const take = (segment: unknown) => {
    if (typeof segment !== "object" || segment === null) throw new Error("invalid display segment")
    const value = segment as Record<string, unknown>
    if (!("data" in value)) return
    const [offset, length] = Array.isArray(value.data) ? value.data : []
    if (offset !== cursor || !Number.isSafeInteger(length) || length < 1 || cursor + length > payload.length) {
      throw new Error("invalid display segment")
    }
    value.data = payload.subarray(cursor, cursor + length)
    cursor += length
  }
  take(frame)
  for (const key of ["tiles", "stripes"]) {
    const list = frame[key]
    if (Array.isArray(list)) for (const segment of list) take(segment)
  }
  if (cursor !== payload.length) throw new Error("display payload not covered")
  return event
}

async function decryptRelayBytes(
  privateKey: CryptoKey,
  payload: EncryptedRelayPayload,
  expectedSenderPublicKey?: string,
): Promise<Uint8Array> {
  // Paired callers supply the kernel key from trusted enrollment, not the envelope.
  // Legacy callers may omit it; decryption alone does not establish sender identity.
  if (expectedSenderPublicKey !== undefined && payload.sender_public_key !== expectedSenderPublicKey) {
    throw new Error("relay sender identity mismatch")
  }
  const nonce = base64ToBytes(payload.nonce)
  if (nonce.byteLength !== relayNonceLength) {
    throw new Error("invalid relay nonce")
  }
  const key = await deriveRelayAesKey(privateKey, payload.sender_public_key)
  const plaintext = await crypto.subtle.decrypt(
    { name: "AES-GCM", iv: bytesToArrayBuffer(nonce) },
    key,
    bytesToArrayBuffer(typeof payload.ciphertext === "string" ? base64ToBytes(payload.ciphertext) : payload.ciphertext),
  )
  return new Uint8Array(plaintext)
}

async function deriveRelayAesKey(privateKey: CryptoKey, peerPublicKeyBase64: string): Promise<CryptoKey> {
  const peerPublicKey = await crypto.subtle.importKey(
    "raw",
    bytesToArrayBuffer(base64ToBytes(peerPublicKeyBase64)),
    { name: "ECDH", namedCurve: "P-256" },
    false,
    [],
  )
  const sharedSecret = await crypto.subtle.deriveBits(
    { name: "ECDH", public: peerPublicKey },
    privateKey,
    256,
  )
  const hkdfKey = await crypto.subtle.importKey("raw", sharedSecret, "HKDF", false, ["deriveKey"])
  return crypto.subtle.deriveKey(
    { name: "HKDF", hash: "SHA-256", salt: new ArrayBuffer(0), info: bytesToArrayBuffer(relayInfo) },
    hkdfKey,
    { name: "AES-GCM", length: 256 },
    false,
    ["encrypt", "decrypt"],
  )
}

function bytesToBase64(bytes: Uint8Array): string {
  let binary = ""
  for (const byte of bytes) {
    binary += String.fromCharCode(byte)
  }
  return btoa(binary)
}

function base64ToBytes(value: string): Uint8Array {
  const binary = atob(value)
  const bytes = new Uint8Array(binary.length)
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index)
  }
  return bytes
}

function bytesToArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  const buffer = new ArrayBuffer(bytes.byteLength)
  new Uint8Array(buffer).set(bytes)
  return buffer
}
