import type { EncryptedRelayPayload as RelayPayload } from "./kernel-transport-frames.js"

// Existing Cloud WebCrypto implementation, exposed for browser-side clients.
const relayNonceLength = 12
const relayInfo = new TextEncoder().encode("chariox-relay-v1")

export type EncryptedRelayPayload = Readonly<RelayPayload>

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
): Promise<{ readonly keypair: RelayKeypair; readonly payload: EncryptedRelayPayload }> {
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
    bytesToArrayBuffer(base64ToBytes(payload.ciphertext)),
  )
  return new TextDecoder().decode(plaintext)
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
