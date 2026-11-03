import { Buffer } from "node:buffer"
export const testRelayKey = Buffer.concat([Buffer.from([4]), Buffer.alloc(64)]).toString("base64")
export function compactRelayTestFrame(header: Record<string, unknown>, payload: Uint8Array = new Uint8Array(32).fill(0xfe)): Buffer {
 const event = String(header.kind).endsWith("event"), id = Buffer.from(String(header.subscription_id ?? header.request_id ?? header.relay_request_id ?? ""))
 const prefix = Buffer.alloc(event ? 92 : 84); prefix.write(event ? "CXR1" : "CXR2"); prefix[4] = String(header.kind).startsWith("client") ? 1 : 0
 prefix.writeUInt16BE(id.length,5); if(event)prefix.writeBigUInt64BE(BigInt(header.event_id as number),7)
 const start=event?15:7; Buffer.from(String(header.sender_public_key),"base64").copy(prefix,start,0,65)
 Buffer.from(String(header.nonce),"base64").copy(prefix,start+65,0,12)
 return Buffer.concat([prefix,id,payload])
}
