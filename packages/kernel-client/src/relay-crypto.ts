import {
  createCipheriv,
  createDecipheriv,
  createECDH,
  hkdfSync,
  randomBytes,
} from "node:crypto"

import type { EncryptedRelayPayload } from "./kernel-transport-frames.js"

const RELAY_NONCE_LEN = 12
const RELAY_TAG_LEN = 16
const RELAY_INFO = Buffer.from("arroba-relay-v1", "utf8")

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

function deriveRelayKey(sharedSecret: Buffer): Buffer {
  return Buffer.from(hkdfSync("sha256", sharedSecret, Buffer.alloc(0), RELAY_INFO, 32))
}
