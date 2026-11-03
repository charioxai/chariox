import { readCompactRoute } from "./relay-binary-routing.js"
import { relayMaxMessageBytes } from "./kernel-direct-types.js"

/** CXL2 uses the sender key already proved by this RTC connection. */
export function expandDirectFrame(data: ArrayBuffer, publicKey: string): string {
  const bytes = new Uint8Array(data), view = new DataView(data)
  const invalid = () => new Error("invalid compact direct frame")
  if (bytes.length < 36 || bytes.length > relayMaxMessageBytes || bytes[0] !== 67 || bytes[1] !== 88 || bytes[2] !== 76 || bytes[3] !== 50 || bytes[4]! > 1) throw invalid()
  const response = bytes[4] === 0, size = view.getUint16(5), start = response ? 19 : 27
  if (size > 1024 || start + (size === 0 ? 16 : size) + 16 > bytes.length) throw invalid()
  const sequence = response ? 0n : view.getBigUint64(19)
  if (sequence > BigInt(Number.MAX_SAFE_INTEGER)) throw invalid()
  const { id, end } = readCompactRoute(bytes, start, size)
  const nonce = btoa(String.fromCharCode(...bytes.subarray(7, 19)))
  const cipher = bytes.subarray(end)
  let binary = ""
  for (let offset = 0; offset < cipher.length; offset += 16384) binary += String.fromCharCode(...cipher.subarray(offset, offset + 16384))
  const encrypted = { sender_public_key: publicKey, nonce, ciphertext: btoa(binary) }
  return JSON.stringify(response ? { kind: "client_response", request_id: id, error: null, encrypted_response: encrypted }
    : { kind: "client_event", subscription_id: id, event_id: Number(sequence), encrypted_event: encrypted })
}
