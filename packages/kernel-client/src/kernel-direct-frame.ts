import { relayMaxMessageBytes } from "./kernel-direct-types.js"

/** CXL1 omits only the sender key already proved by this RTC connection. */
export function expandDirectFrame(data: ArrayBuffer, publicKey: string): string {
  const bytes = new Uint8Array(data), view = new DataView(data)
  const invalid = () => new Error("invalid compact direct frame")
  if (bytes.length < 26 || bytes.length > relayMaxMessageBytes || bytes[0] !== 67 || bytes[1] !== 88 || bytes[2] !== 76 || bytes[3] !== 49) throw invalid()
  const size = view.getUint32(4)
  if (size < 2 || size > 4096 || size + 24 > bytes.length) throw invalid()
  const header: unknown = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes.subarray(8, 8 + size)))
  if (!header || typeof header !== "object" || Array.isArray(header)) throw invalid()
  const h = header as Record<string, unknown>, response = typeof h.r === "string"
  const id = response ? h.r : h.s
  if (typeof id !== "string" || !id || new TextEncoder().encode(id).length > 1024 || typeof h.n !== "string" || !/^[A-Za-z0-9+/]{16}$/.test(h.n)
    || Object.keys(h).sort().join(",") !== (response ? "n,r" : "e,n,s")
    || (!response && (!Number.isSafeInteger(h.e) || (h.e as number) < 0))) throw invalid()
  const cipher = bytes.subarray(8 + size)
  let binary = ""
  for (let offset = 0; offset < cipher.length; offset += 16384) binary += String.fromCharCode(...cipher.subarray(offset, offset + 16384))
  const encrypted = { sender_public_key: publicKey, nonce: h.n, ciphertext: btoa(binary) }
  return JSON.stringify(response ? { kind: "client_response", request_id: id, error: null, encrypted_response: encrypted }
    : { kind: "client_event", subscription_id: id, event_id: h.e, encrypted_event: encrypted })
}
