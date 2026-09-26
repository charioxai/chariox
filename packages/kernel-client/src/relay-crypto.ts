import {
  createHash,
  createCipheriv,
  createDecipheriv,
  createECDH,
  hkdfSync,
  randomBytes,
} from "node:crypto"

import type { EncryptedRelayPayload } from "./kernel-transport-frames.js"

const RELAY_NONCE_LEN = 12
const RELAY_TAG_LEN = 16
const RELAY_INFO = Buffer.from("chariox-relay-v1", "utf8")

export function encryptRelayPayload(
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

export function decryptRelayPayload(privateKey: Buffer, payload: EncryptedRelayPayload): string {
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

/**
 * A paired CLI relay identity. The private key remains in a closure-backed
 * private field; callers can use it for relay and display crypto without
 * serializing or logging the key itself.
 */
export class RelayClientIdentity {
  readonly publicKeyBase64: string
  readonly publicKeyThumbprint: string
  readonly #privateKey: Buffer

  constructor(privateKey: Buffer) {
    this.#privateKey = Buffer.from(privateKey)
    this.publicKeyBase64 = relayPublicKeyFromPrivateKey(this.#privateKey)
    this.publicKeyThumbprint = relayPublicKeyThumbprint(this.publicKeyBase64)
  }

  encrypt(peerPublicKeyBase64: string, plaintext: Buffer | string): EncryptedRelayPayload {
    return encryptRelayPayloadWithKeypair(
      peerPublicKeyBase64,
      Buffer.isBuffer(plaintext) ? plaintext : Buffer.from(plaintext, "utf8"),
      this.#privateKey,
      this.publicKeyBase64,
    )
  }

  decrypt(payload: EncryptedRelayPayload, expectedSenderPublicKey?: string): string {
    if (expectedSenderPublicKey !== undefined && payload.sender_public_key !== expectedSenderPublicKey) {
      throw new Error("relay sender identity mismatch")
    }
    return decryptRelayPayload(this.#privateKey, payload)
  }
}

export function relayPublicKeyThumbprint(publicKeyBase64: string): string {
  return createHash("sha256").update(publicKeyBase64, "utf8").digest("hex")
}

export function createRelayKeypair(): { privateKey: Buffer; publicKeyBase64: string } {
  const ecdh = createECDH("prime256v1")
  const publicKey = ecdh.generateKeys()
  return {
    privateKey: ecdh.getPrivateKey(),
    publicKeyBase64: publicKey.toString("base64"),
  }
}

export function relayPublicKeyFromPrivateKey(privateKey: Buffer): string {
  const ecdh = createECDH("prime256v1")
  ecdh.setPrivateKey(privateKey)
  return ecdh.getPublicKey().toString("base64")
}

function encryptRelayPayloadWithKeypair(
  peerPublicKeyBase64: string,
  plaintext: Buffer,
  privateKey: Buffer,
  publicKeyBase64: string,
): EncryptedRelayPayload {
  const ecdh = createECDH("prime256v1")
  ecdh.setPrivateKey(privateKey)
  const sharedSecret = ecdh.computeSecret(Buffer.from(peerPublicKeyBase64, "base64"))
  const key = deriveRelayKey(sharedSecret)
  const nonce = randomBytes(RELAY_NONCE_LEN)
  const cipher = createCipheriv("aes-256-gcm", key, nonce)
  const ciphertext = Buffer.concat([cipher.update(plaintext), cipher.final(), cipher.getAuthTag()])
  return {
    sender_public_key: publicKeyBase64,
    nonce: nonce.toString("base64"),
    ciphertext: ciphertext.toString("base64"),
  }
}

function deriveRelayKey(sharedSecret: Buffer): Buffer {
  return Buffer.from(hkdfSync("sha256", sharedSecret, Buffer.alloc(0), RELAY_INFO, 32))
}
